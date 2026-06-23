use std::process::Command;

fn agent() -> Command {
    Command::new(std::env::var("CARGO_BIN_EXE_devrelay-agent").unwrap())
}

#[test]
fn foreground_health_smoke_test_loads_config_and_migrates_database() {
    let root = std::env::temp_dir().join(format!("devrelay-agent-test-{}", std::process::id()));
    let config = root.join("config.toml");
    let socket = root.join("agent.sock");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();

    let output = agent()
        .env("DEVRELAY_HOME", &root)
        .args([
            "--foreground",
            "--config",
            config.to_str().unwrap(),
            "--socket-path",
            socket.to_str().unwrap(),
            "--log-level",
            "debug",
            "--health",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let health: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(health["status"].as_str(), Some("ok"));
    assert_eq!(health["foreground"].as_bool(), Some(true));
    assert_eq!(health["project_count"].as_u64(), Some(0));
    assert!(config.exists());
    assert!(root.join("agent.sqlite").exists());

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn foreground_binds_configured_ipc_socket() {
    let mut running = RunningAgent::start("devrelay-agent-ipc-test");

    let stream = std::os::unix::net::UnixStream::connect(&running.socket).unwrap();
    drop(stream);

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_serves_rpc_negotiate_and_agent_health() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-rpc-test");

    let negotiate = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "negotiate-1",
            "method": "rpc.negotiate",
            "params": { "client_protocol_version": 1 }
        }),
    );
    assert_eq!(negotiate["id"], "negotiate-1");
    assert_eq!(negotiate["result"]["protocol_version"], 1);
    assert_eq!(negotiate["result"]["server_name"], "devrelay-agent");
    assert!(
        negotiate["result"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "agent.health")
    );

    let health = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "agent.health"
        }),
    );
    assert_eq!(health["id"], 7);
    assert_eq!(health["result"]["status"], "ok");
    assert_eq!(health["result"]["foreground"], true);
    assert_eq!(
        health["result"]["socket_path"].as_str(),
        Some(running.socket.to_str().unwrap())
    );
    assert!(health.get("error").is_none());

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_serves_status_get_rpc() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-status-rpc-test");
    let repo = running.root.join("repo");
    std::fs::create_dir(&repo).unwrap();
    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "devrelay@example.test"]);
    run_git(&repo, &["config", "user.name", "DevRelay Test"]);
    std::fs::write(repo.join("README.md"), "base\n").unwrap();
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-m", "base"]);
    std::fs::write(
        repo.join("devrelay.toml"),
        r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
    )
    .unwrap();
    run_git(&repo, &["add", "devrelay.toml"]);
    run_git(&repo, &["commit", "-m", "add manifest"]);
    std::fs::write(repo.join("notes.md"), "untracked\n").unwrap();

    let status = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "status-1",
            "method": "status.get",
            "params": {
                "repo": repo,
                "manifest": repo.join("devrelay.toml")
            }
        }),
    );

    assert_eq!(status["id"], "status-1");
    assert_eq!(status["result"]["status"]["branch"], "main");
    assert_eq!(status["result"]["status"]["counts"]["untracked"], 1);
    assert!(
        status["result"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == "notes.md")
    );
    assert!(
        status["result"]["untracked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == "notes.md" && entry["decision"] == "include")
    );
    assert!(status.get("error").is_none());

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_serves_project_registry_rpc() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-project-rpc-test");
    let repo = running.root.join("project");
    std::fs::create_dir(&repo).unwrap();
    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "devrelay@example.test"]);
    run_git(&repo, &["config", "user.name", "DevRelay Test"]);
    std::fs::write(repo.join("README.md"), "base\n").unwrap();
    std::fs::write(
        repo.join("devrelay.toml"),
        r#"
schema = 1
project_id = "87654321"
name = "Demo Project"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
    )
    .unwrap();
    run_git(&repo, &["add", "README.md", "devrelay.toml"]);
    run_git(&repo, &["commit", "-m", "base"]);

    let added = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "add-1",
            "method": "projects.add",
            "params": {
                "path": repo,
                "manifest": repo.join("devrelay.toml")
            }
        }),
    );
    assert_eq!(added["id"], "add-1");
    assert_eq!(added["result"]["project"]["project_id"], "87654321");
    assert_eq!(added["result"]["project"]["display_name"], "Demo Project");
    assert_eq!(
        added["result"]["project"]["workspaces"]
            .as_object()
            .unwrap()
            .len(),
        1
    );

    let listed = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "list-1",
            "method": "projects.list"
        }),
    );
    assert_eq!(
        listed["result"]["projects"][0]["display_name"],
        "Demo Project"
    );

    let shown = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "show-1",
            "method": "projects.show",
            "params": { "id_or_name": "Demo Project" }
        }),
    );
    assert_eq!(shown["result"]["project"]["project_id"], "87654321");

    let removed = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "remove-1",
            "method": "projects.remove",
            "params": { "id_or_name": "Demo Project" }
        }),
    );
    assert_eq!(removed["result"]["project"]["project_id"], "87654321");

    let listed_after_remove = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "list-2",
            "method": "projects.list"
        }),
    );
    assert!(
        listed_after_remove["result"]["projects"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let health = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "health-projects",
            "method": "agent.health"
        }),
    );
    assert_eq!(health["result"]["project_count"], 0);
    assert!(running.root.join("config.toml").exists());

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_serves_checkpoint_create_and_snapshots_list_rpc() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-snapshot-rpc-test");
    let repo = running.root.join("snapshot-project");
    create_manifest_repo(&repo, "11223344", "Snapshot Project");
    std::fs::write(repo.join("README.md"), "base\nchanged\n").unwrap();
    std::fs::write(repo.join("notes.md"), "carry me\n").unwrap();

    let checkpoint = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "checkpoint-1",
            "method": "checkpoint.create",
            "params": {
                "repo": repo,
                "manifest": repo.join("devrelay.toml"),
                "label": "rpc checkpoint",
                "pin": true
            }
        }),
    );
    assert_eq!(checkpoint["id"], "checkpoint-1");
    assert_eq!(checkpoint["result"]["checkpoint"]["project_id"], "11223344");
    assert_eq!(checkpoint["result"]["checkpoint"]["pinned"], true);
    assert_eq!(
        checkpoint["result"]["checkpoint"]["label"],
        "rpc checkpoint"
    );
    assert!(
        checkpoint["result"]["checkpoint"]["metadata"]["included_untracked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry == "notes.md")
    );
    assert!(std::path::Path::new(checkpoint["result"]["snapshot_repo"].as_str().unwrap()).exists());

    let snapshot_id = checkpoint["result"]["checkpoint"]["snapshot_id"]
        .as_str()
        .unwrap();
    let listed = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "snapshots-1",
            "method": "snapshots.list",
            "params": { "project": "11223344" }
        }),
    );
    assert_eq!(listed["result"]["snapshots"][0]["snapshot_id"], snapshot_id);
    assert_eq!(listed["result"]["snapshots"][0]["sequence_number"], 1);

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_serves_apply_snapshot_rpc() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-apply-rpc-test");
    let source = running.root.join("apply-source");
    let target = running.root.join("apply-target");
    create_manifest_repo(&source, "55667788", "Apply Project");
    std::fs::write(source.join("README.md"), "base\nchanged\n").unwrap();
    std::fs::write(source.join("notes.md"), "carry me\n").unwrap();
    clone_repo(&source, &target);

    let checkpoint = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "checkpoint-apply",
            "method": "checkpoint.create",
            "params": {
                "repo": source,
                "manifest": source.join("devrelay.toml"),
                "label": "apply source"
            }
        }),
    );
    let snapshot_id = checkpoint["result"]["checkpoint"]["snapshot_id"]
        .as_str()
        .unwrap();

    let dry_run = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "apply-dry-run",
            "method": "apply.snapshot",
            "params": {
                "repo": target,
                "project": "55667788",
                "snapshot_id": snapshot_id,
                "dry_run": true
            }
        }),
    );
    assert_eq!(dry_run["id"], "apply-dry-run");
    assert_eq!(dry_run["result"]["plan"]["snapshot_id"], snapshot_id);
    assert!(dry_run["result"]["verification"].is_null());
    assert_eq!(
        std::fs::read_to_string(target.join("README.md")).unwrap(),
        "base\n"
    );
    assert!(!target.join("notes.md").exists());

    let applied = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "apply-1",
            "method": "apply.snapshot",
            "params": {
                "repo": target,
                "project": "55667788",
                "snapshot_id": snapshot_id
            }
        }),
    );
    assert_eq!(applied["id"], "apply-1");
    assert_eq!(applied["result"]["snapshot"]["snapshot_id"], snapshot_id);
    assert_eq!(
        applied["result"]["verification"]["included_untracked"][0],
        "notes.md"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("README.md")).unwrap(),
        "base\nchanged\n"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("notes.md")).unwrap(),
        "carry me\n"
    );

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_serves_recover_open_rpc() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-recover-rpc-test");
    let source = running.root.join("recover-source");
    let recovered = running.root.join("recovered-workspace");
    create_manifest_repo(&source, "66778899", "Recover Project");

    let added = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "recover-project-add",
            "method": "projects.add",
            "params": {
                "path": source,
                "manifest": source.join("devrelay.toml")
            }
        }),
    );
    assert_eq!(added["result"]["project"]["project_id"], "66778899");

    std::fs::write(source.join("README.md"), "base\nrecovered\n").unwrap();
    std::fs::write(source.join("notes.md"), "recover me\n").unwrap();
    let checkpoint = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "recover-checkpoint",
            "method": "checkpoint.create",
            "params": {
                "repo": source,
                "manifest": source.join("devrelay.toml"),
                "label": "recover source"
            }
        }),
    );
    let snapshot_id = checkpoint["result"]["checkpoint"]["snapshot_id"]
        .as_str()
        .unwrap();

    let listed = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "recover-list",
            "method": "recover.list",
            "params": { "project": "Recover Project" }
        }),
    );
    assert_eq!(
        listed["result"]["snapshots"][0]["snapshot_id"].as_str(),
        Some(snapshot_id)
    );

    let recover_show = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "recover-show",
            "method": "recover.show",
            "params": {
                "snapshot_id": snapshot_id,
                "project": "Recover Project"
            }
        }),
    );
    assert_eq!(
        recover_show["result"]["snapshot"]["snapshot_id"].as_str(),
        Some(snapshot_id)
    );

    let opened = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "recover-open",
            "method": "recover.open",
            "params": {
                "snapshot_id": snapshot_id,
                "path": recovered,
                "project": "Recover Project",
                "register": true,
                "name": "Recovered copy"
            }
        }),
    );
    assert_eq!(opened["id"], "recover-open");
    assert_eq!(opened["result"]["recovered"]["snapshot_id"], snapshot_id);
    assert_eq!(opened["result"]["name"], "Recovered copy");
    assert_eq!(opened["result"]["registered"]["project_id"], "66778899");
    assert_eq!(
        opened["result"]["verification"]["included_untracked"][0],
        "notes.md"
    );
    assert_eq!(
        std::fs::read_to_string(recovered.join("README.md")).unwrap(),
        "base\nrecovered\n"
    );
    assert_eq!(
        std::fs::read_to_string(recovered.join("notes.md")).unwrap(),
        "recover me\n"
    );
    assert_eq!(
        std::fs::read_to_string(source.join("README.md")).unwrap(),
        "base\nrecovered\n"
    );

    let shown = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "recover-project-show",
            "method": "projects.show",
            "params": { "id_or_name": "66778899" }
        }),
    );
    assert_eq!(
        shown["result"]["project"]["workspaces"]
            .as_object()
            .unwrap()
            .len(),
        2
    );

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_serves_diagnostics_export_rpc() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-diagnostics-rpc-test");
    let out = running.root.join("diagnostics").join("bundle.json");

    let exported = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "diagnostics-1",
            "method": "diagnostics.export",
            "params": { "out": out }
        }),
    );

    assert_eq!(exported["id"], "diagnostics-1");
    assert_eq!(
        exported["result"]["path"].as_str(),
        Some(out.to_str().unwrap())
    );
    assert_eq!(exported["result"]["source_code_included"], false);
    assert_eq!(exported["result"]["snapshot_objects_included"], false);
    let bundle: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(bundle["health"]["status"], "ok");
    assert_eq!(bundle["source_code_included"], false);
    assert_eq!(bundle["snapshot_objects_included"], false);
    assert!(
        bundle["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "diagnostics.export")
    );

    running.stop();
}

#[cfg(unix)]
fn rpc_call(
    connection: &mut devrelay_core::UnixIpcConnection,
    request: serde_json::Value,
) -> serde_json::Value {
    use devrelay_core::{IpcConnection, IpcLimits};

    let request = serde_json::to_vec(&request).unwrap();
    connection
        .write_message(&request, IpcLimits::default())
        .unwrap();
    let response = connection.read_message(IpcLimits::default()).unwrap();
    serde_json::from_slice(&response).unwrap()
}

#[cfg(unix)]
fn run_git(repo: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

#[cfg(unix)]
fn clone_repo(source: &std::path::Path, target: &std::path::Path) {
    let status = Command::new("git")
        .arg("clone")
        .arg(source)
        .arg(target)
        .status()
        .unwrap();
    assert!(status.success(), "git clone failed");
}

#[cfg(unix)]
fn create_manifest_repo(repo: &std::path::Path, project_id: &str, name: &str) {
    std::fs::create_dir(repo).unwrap();
    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "devrelay@example.test"]);
    run_git(repo, &["config", "user.name", "DevRelay Test"]);
    std::fs::write(repo.join("README.md"), "base\n").unwrap();
    std::fs::write(
        repo.join("devrelay.toml"),
        format!(
            r#"
schema = 1
project_id = "{project_id}"
name = "{name}"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#
        ),
    )
    .unwrap();
    run_git(repo, &["add", "README.md", "devrelay.toml"]);
    run_git(repo, &["commit", "-m", "base"]);
}

#[cfg(unix)]
struct RunningAgent {
    root: std::path::PathBuf,
    socket: std::path::PathBuf,
    child: std::process::Child,
}

#[cfg(unix)]
impl RunningAgent {
    fn start(name: &str) -> Self {
        use std::os::unix::fs::FileTypeExt;
        use std::process::Stdio;
        use std::time::{Duration, Instant};

        let root = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let config = root.join("config.toml");
        let socket = root.join("agent.sock");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();

        let mut child = agent()
            .env("DEVRELAY_HOME", &root)
            .args([
                "--foreground",
                "--config",
                config.to_str().unwrap(),
                "--socket-path",
                socket.to_str().unwrap(),
            ])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Ok(metadata) = std::fs::symlink_metadata(&socket)
                && metadata.file_type().is_socket()
            {
                return Self {
                    root,
                    socket,
                    child,
                };
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        child.kill().ok();
        let output = child.wait_with_output().unwrap();
        panic!(
            "agent did not bind socket {}; stderr={}",
            socket.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn stop(&mut self) {
        self.child.kill().ok();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
impl Drop for RunningAgent {
    fn drop(&mut self) {
        self.stop();
    }
}
