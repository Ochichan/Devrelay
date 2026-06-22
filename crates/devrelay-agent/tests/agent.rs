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
