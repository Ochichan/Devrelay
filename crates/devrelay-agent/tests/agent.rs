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
