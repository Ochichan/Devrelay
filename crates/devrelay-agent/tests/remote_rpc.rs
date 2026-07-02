//! Integration tests for the remote Control RPC server over mTLS.
//!
//! These tests spawn the real agent binary with `--remote-listen`, issue
//! fabric credentials for a second device, and drive the implemented API
//! boundary end to end: authenticated reads, the method allowlist,
//! unauthenticated rejection, revocation, and the remote recovery flow.

#![cfg(unix)]

use devrelay_core::{
    AgentRpcClient, CheckpointCreateParams, CheckpointCreateResult, ControlPlaneTransportPolicy,
    DevRelayHome, DeviceCertificate, DevicePublicIdentity, FabricIdentityStore, IpcLimits,
    LocalConfig, MetadataDb, ProjectResult, ProjectsAddParams, RemoteControlClient, RustlsIdentity,
    build_rustls_client_config, ed25519_seed_to_pkcs8_der, unix_now_seconds,
};
use serde_json::json;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct AgentUnderTest {
    child: Child,
    root: PathBuf,
    remote_address: SocketAddr,
}

impl AgentUnderTest {
    fn start(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("devrelay-remote-rpc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let child = Command::new(std::env::var("CARGO_BIN_EXE_devrelay-agent").unwrap())
            .env("DEVRELAY_HOME", &root)
            .args([
                "--foreground",
                "--config",
                root.join("config.toml").to_str().unwrap(),
                "--socket-path",
                root.join("agent.sock").to_str().unwrap(),
                "--remote-listen",
                "127.0.0.1:0",
                "--log-level",
                "debug",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let address_file = DevRelayHome::new(&root).agent_remote_address_path();
        let deadline = Instant::now() + Duration::from_secs(20);
        let remote_address = loop {
            if let Ok(raw) = std::fs::read_to_string(&address_file)
                && let Ok(address) = raw.trim().parse::<SocketAddr>()
            {
                break address;
            }
            assert!(
                Instant::now() < deadline,
                "agent did not write {}",
                address_file.display()
            );
            std::thread::sleep(Duration::from_millis(25));
        };
        Self {
            child,
            root,
            remote_address,
        }
    }

    fn home(&self) -> DevRelayHome {
        DevRelayHome::new(&self.root)
    }

    fn identity_store(&self) -> FabricIdentityStore {
        FabricIdentityStore::new(self.home())
    }

    fn server_signing_public_key_hex(&self) -> String {
        self.identity_store()
            .public_bundle_from_store(&LocalConfig::default())
            .unwrap()
            .device
            .signing_public_key_hex
    }

    fn issue_client_credentials(
        &self,
        device_id: &str,
        seed: [u8; 32],
    ) -> (RustlsIdentity, DeviceCertificate, Vec<u8>) {
        let store = self.identity_store();
        let bundle = store
            .public_bundle_from_store(&LocalConfig::default())
            .unwrap();
        let public_key_hex = hex_encode(
            &ed25519_dalek::SigningKey::from_bytes(&seed)
                .verifying_key()
                .to_bytes(),
        );
        let leaf_der = store
            .issue_peer_tls_certificate_der(device_id, &public_key_hex)
            .unwrap();
        let identity = RustlsIdentity {
            cert_chain_der: vec![leaf_der],
            private_key_pkcs8_der: ed25519_seed_to_pkcs8_der(&seed),
        };
        let peer_identity = DevicePublicIdentity {
            device_id: device_id.to_string(),
            display_name: format!("Test peer {device_id}"),
            fabric_id: bundle.root.fabric_id.clone(),
            signing_public_key_hex: public_key_hex,
            network_public_key_hex: "b".repeat(64),
            platform_key: "test".to_string(),
            architecture: "test".to_string(),
            created_at_unix_seconds: unix_now_seconds(),
            last_seen_unix_seconds: unix_now_seconds(),
        };
        let certificate = store
            .issue_device_certificate(&peer_identity, unix_now_seconds() - 60, 24 * 3_600)
            .unwrap();
        let fabric_ca = store.fabric_tls_ca_der().unwrap();
        (identity, certificate, fabric_ca)
    }

    fn connect_client(&self, device_id: &str, seed: [u8; 32]) -> RemoteControlClient {
        let (identity, certificate, fabric_ca) = self.issue_client_credentials(device_id, seed);
        let tls_config = build_rustls_client_config(identity, vec![fabric_ca]).unwrap();
        RemoteControlClient::connect(
            self.remote_address,
            tls_config,
            certificate,
            Some(&self.server_signing_public_key_hex()),
            &ControlPlaneTransportPolicy::default(),
            IpcLimits::default(),
        )
        .unwrap()
    }

    fn local_client(&self) -> AgentRpcClient {
        AgentRpcClient::new(self.root.join("agent.sock"))
    }
}

impl Drop for AgentUnderTest {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "DevRelay Test")
        .env("GIT_AUTHOR_EMAIL", "test@devrelay.invalid")
        .env("GIT_COMMITTER_NAME", "DevRelay Test")
        .env("GIT_COMMITTER_EMAIL", "test@devrelay.invalid")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn remote_control_api_serves_authenticated_reads_and_enforces_allowlist() {
    let agent = AgentUnderTest::start("reads");
    let mut client = agent.connect_client("remote-reader", [42u8; 32]);

    let negotiate = client
        .call("rpc.negotiate", json!({"client_protocol_version": 1}))
        .unwrap();
    let result = negotiate.result.expect("negotiate result");
    assert_eq!(result["server_name"], "devrelay-remote");
    assert_eq!(result["protocol_version"], 1);
    let methods = result["methods"].as_array().unwrap();
    assert!(methods.iter().any(|method| method == "devices.list"));
    assert!(methods.iter().any(|method| method == "recovery.open"));
    assert!(!methods.iter().any(|method| method == "settings.update"));

    let devices = client.call("devices.list", json!({})).unwrap();
    let devices = devices.result.expect("devices result");
    assert!(
        !devices["devices"].as_array().unwrap().is_empty(),
        "agent device identity should be listed"
    );

    let projects = client.call("projects.list", json!({})).unwrap();
    assert_eq!(
        projects.result.expect("projects result")["projects"],
        json!([])
    );

    // Local-only methods stay outside the remote boundary.
    let forbidden = client.call("settings.update", json!({})).unwrap();
    assert_eq!(forbidden.error.expect("allowlist error").code, -32601);

    // The same connection keeps serving after a rejected request.
    let after = client.call("devices.list", json!({})).unwrap();
    assert!(after.error.is_none());
}

#[test]
fn remote_control_api_rejects_unauthenticated_and_revoked_peers() {
    let agent = AgentUnderTest::start("authz");

    // A TLS client without a certificate must not reach method dispatch.
    let (_, certificate, fabric_ca) = agent.issue_client_credentials("no-cert-client", [44u8; 32]);
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(fabric_ca))
        .unwrap();
    let no_auth_config = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let unauthenticated = RemoteControlClient::connect(
        agent.remote_address,
        no_auth_config,
        certificate,
        None,
        &ControlPlaneTransportPolicy::default(),
        IpcLimits::default(),
    );
    let rejected = match unauthenticated {
        // TLS 1.3 clients may believe the handshake finished before the
        // server rejects the missing certificate; the first request must
        // then fail instead.
        Ok(mut client) => client.call("devices.list", json!({})).is_err(),
        Err(_) => true,
    };
    assert!(rejected, "certificate-less peer must be rejected");

    // A revoked device is rejected before dispatch even with valid TLS.
    let mut revoked_client = agent.connect_client("revoked-client", [43u8; 32]);
    let mut db = MetadataDb::open(agent.root.join("agent.sqlite")).unwrap();
    db.revoke_device("revoked-client", "test-suite", "lost device", false)
        .unwrap();
    let response = revoked_client.call("devices.list", json!({})).unwrap();
    let error = response.error.expect("revoked device error");
    assert!(
        error.data.unwrap()["detail"]
            .as_str()
            .unwrap()
            .contains("revoked")
    );

    // A healthy device still works after the revoked peer was blocked.
    let mut client = agent.connect_client("healthy-client", [42u8; 32]);
    let devices = client.call("devices.list", json!({})).unwrap();
    assert!(devices.error.is_none());
}

#[test]
fn remote_recovery_flow_lists_and_opens_snapshots() {
    let agent = AgentUnderTest::start("recovery");

    // Register a real project and checkpoint dirty work through local RPC.
    let repo = agent.root.join("demo-project");
    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "--initial-branch=main"]);
    std::fs::write(
        repo.join("devrelay.toml"),
        concat!(
            "schema = 1\n",
            "project_id = \"remote-rpc-demo-0001\"\n",
            "name = \"remote-rpc-demo\"\n\n",
            "[workspace]\n",
            "untracked = \"safe\"\n",
            "portable_paths = \"strict\"\n",
        ),
    )
    .unwrap();
    std::fs::write(repo.join("main.txt"), "committed\n").unwrap();
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "initial"]);
    std::fs::write(repo.join("main.txt"), "committed\nwork in progress\n").unwrap();

    let local = agent.local_client();
    let project: ProjectResult = local
        .call(
            "projects.add",
            ProjectsAddParams {
                path: repo.clone(),
                manifest: None,
            },
        )
        .unwrap();
    let project_id = project.project.project_id.clone();
    let checkpoint: CheckpointCreateResult = local
        .call(
            "checkpoint.create",
            CheckpointCreateParams {
                repo: repo.clone(),
                manifest: None,
                label: Some("before handoff".to_string()),
                pin: false,
            },
        )
        .unwrap();
    let snapshot_id = checkpoint.checkpoint.snapshot_id.clone();

    let mut client = agent.connect_client("recovery-client", [42u8; 32]);
    let listed = client
        .call("recovery.list", json!({"project": project_id}))
        .unwrap();
    let listed = listed.result.expect("recovery list result");
    let snapshots = listed["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0]["snapshot_id"], snapshot_id.as_str());
    assert_eq!(snapshots[0]["label"], "before handoff");
    assert!(snapshots[0].get("metadata").is_none());

    let recovered_path = agent.root.join("recovered-workspace");
    let opened = client
        .call(
            "recovery.open",
            json!({
                "snapshot_id": snapshot_id,
                "project": project_id,
                "path": recovered_path,
            }),
        )
        .unwrap();
    let opened = opened.result.expect("recovery open result");
    assert_eq!(opened["recovered"]["snapshot_id"], snapshot_id.as_str());
    assert!(opened["registered"].is_null());
    assert!(opened["verification"]["state_hash"].as_str().unwrap().len() > 8);
    let recovered = std::fs::read_to_string(recovered_path.join("main.txt")).unwrap();
    assert_eq!(recovered, "committed\nwork in progress\n");

    // Unknown projects are rejected with a structured error.
    let missing = client
        .call("recovery.list", json!({"project": "missing"}))
        .unwrap();
    assert!(missing.error.is_some());
}
