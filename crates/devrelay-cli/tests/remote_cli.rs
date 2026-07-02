//! End-to-end CLI flow for remote Control RPC access.
//!
//! Covers the human path from pairing to a remote call: the fabric owner
//! confirms a pairing session and issues a credential bundle, the peer
//! imports it into its own DEVRELAY_HOME, and `devrelay remote call` drives
//! the mTLS Control API of the owner's agent.

#![cfg(unix)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn cli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_devrelay"))
}

fn agent_binary() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_devrelay-agent")
        .map(PathBuf::from)
        .unwrap_or_else(|| cli_binary().with_file_name("devrelay-agent"))
}

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new(cli_binary())
        .env("DEVRELAY_HOME", home)
        .args(args)
        .output()
        .unwrap()
}

fn cli_ok_json(home: &Path, args: &[&str]) -> serde_json::Value {
    let output = cli(home, args);
    assert!(
        output.status.success(),
        "devrelay {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "devrelay {args:?} produced non-JSON output ({err}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

struct AgentProcess {
    child: Child,
    remote_address: SocketAddr,
}

fn start_agent(home: &Path) -> AgentProcess {
    assert!(
        agent_binary().exists(),
        "devrelay-agent binary not found at {}; run cargo test --workspace",
        agent_binary().display()
    );
    let child = Command::new(agent_binary())
        .env("DEVRELAY_HOME", home)
        .args([
            "--foreground",
            "--config",
            home.join("config.toml").to_str().unwrap(),
            "--socket-path",
            home.join("agent.sock").to_str().unwrap(),
            "--remote-listen",
            "127.0.0.1:0",
            "--log-level",
            "error",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let address_file = home.join("agent-remote.addr");
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
    AgentProcess {
        child,
        remote_address,
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn paired_device_reaches_remote_control_api_through_cli_credentials() {
    let root = std::env::temp_dir().join(format!("devrelay-remote-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let owner_home = root.join("owner");
    let peer_home = root.join("peer");
    std::fs::create_dir_all(&owner_home).unwrap();
    std::fs::create_dir_all(&peer_home).unwrap();

    let agent = start_agent(&owner_home);

    // Peer identity provides the public keys the owner pairs against.
    let peer_identity = cli_ok_json(&peer_home, &["identity", "init", "--json"]);
    let peer_device_id = peer_identity["device"]["device_id"].as_str().unwrap();
    let peer_signing_key = peer_identity["device"]["signing_public_key_hex"]
        .as_str()
        .unwrap();
    let peer_network_key = peer_identity["device"]["network_public_key_hex"]
        .as_str()
        .unwrap();

    // Owner pairs and confirms with the displayed short authentication code.
    let ephemeral = "ab".repeat(32);
    let session = cli_ok_json(
        &owner_home,
        &[
            "pairing",
            "start",
            "--peer-device-id",
            peer_device_id,
            "--peer-name",
            "Remote CLI Peer",
            "--peer-signing-public-key",
            peer_signing_key,
            "--peer-network-public-key",
            peer_network_key,
            "--peer-ephemeral-public-key",
            &ephemeral,
            "--json",
        ],
    );
    let pairing_id = session["pairing_id"].as_str().unwrap();
    let code = session["short_authentication_string"].as_str().unwrap();
    let confirmed = cli_ok_json(
        &owner_home,
        &["pairing", "confirm", pairing_id, "--code", code, "--json"],
    );
    assert_eq!(confirmed["state"], "confirmed");

    // Owner issues the credential bundle; the peer imports it.
    let bundle_path = root.join("peer-remote-credentials.json");
    let issue = cli(
        &owner_home,
        &[
            "remote",
            "credentials",
            "issue",
            pairing_id,
            "--out",
            bundle_path.to_str().unwrap(),
        ],
    );
    assert!(
        issue.status.success(),
        "credential issue failed: {}",
        String::from_utf8_lossy(&issue.stderr)
    );
    let imported = cli_ok_json(
        &peer_home,
        &[
            "remote",
            "credentials",
            "import",
            bundle_path.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(imported["subject_device_id"], peer_device_id);

    // The peer can now call the remote Control API over mTLS.
    let address = agent.remote_address.to_string();
    let devices = cli_ok_json(
        &peer_home,
        &[
            "remote",
            "call",
            "devices.list",
            "--address",
            &address,
            "--json",
        ],
    );
    assert!(devices["error"].is_null());
    assert!(
        !devices["result"]["devices"].as_array().unwrap().is_empty(),
        "owner agent device should be listed"
    );

    // Local-only methods stay rejected at the remote boundary.
    let forbidden = cli(
        &peer_home,
        &["remote", "call", "settings.update", "--address", &address],
    );
    assert!(!forbidden.status.success());
    assert!(
        String::from_utf8_lossy(&forbidden.stderr).contains("-32601")
            || String::from_utf8_lossy(&forbidden.stderr).contains("Method not found"),
        "unexpected failure output: {}",
        String::from_utf8_lossy(&forbidden.stderr)
    );

    // A device that never imported credentials cannot call at all.
    let stranger_home = root.join("stranger");
    std::fs::create_dir_all(&stranger_home).unwrap();
    cli_ok_json(&stranger_home, &["identity", "init", "--json"]);
    let unauthorized = cli(
        &stranger_home,
        &["remote", "call", "devices.list", "--address", &address],
    );
    assert!(!unauthorized.status.success());
    assert!(
        String::from_utf8_lossy(&unauthorized.stderr).contains("no remote access credentials"),
        "unexpected stranger output: {}",
        String::from_utf8_lossy(&unauthorized.stderr)
    );

    drop(agent);
    let _ = std::fs::remove_dir_all(&root);
}
