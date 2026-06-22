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
    use std::os::unix::fs::FileTypeExt;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let root = std::env::temp_dir().join(format!("devrelay-agent-ipc-test-{}", std::process::id()));
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
            let stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
            drop(stream);
            child.kill().ok();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(root);
            return;
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
