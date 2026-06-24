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
    assert_eq!(health["role"].as_str(), Some("local-only"));
    assert_eq!(health["anchor_mode"].as_str(), Some("local-only"));
    assert!(health["anchor"].is_null());
    assert_eq!(health["foreground"].as_bool(), Some(true));
    assert_eq!(health["project_count"].as_u64(), Some(0));
    assert!(config.exists());
    assert!(root.join("agent.sqlite").exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn foreground_health_reports_anchor_role_from_config() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-agent-anchor-health-test-{}",
        std::process::id()
    ));
    let config = root.join("config.toml");
    let socket = root.join("agent.sock");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();

    let local_config = devrelay_core::LocalConfig {
        anchor_mode: devrelay_core::AnchorMode::UserSelected,
        ..devrelay_core::LocalConfig::default()
    };
    local_config.save(&config).unwrap();

    let output = agent()
        .env("DEVRELAY_HOME", &root)
        .args([
            "--foreground",
            "--config",
            config.to_str().unwrap(),
            "--socket-path",
            socket.to_str().unwrap(),
            "--health",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let health: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(health["status"], "ok");
    assert_eq!(health["role"], "anchor");
    assert_eq!(health["anchor_mode"], "user-selected");
    assert_eq!(
        health["database_path"].as_str(),
        Some(
            root.join("anchor")
                .join("metadata.sqlite")
                .to_str()
                .unwrap()
        )
    );
    assert_eq!(
        health["anchor"]["startup_path"].as_str(),
        Some(root.join("anchor").join("startup.json").to_str().unwrap())
    );
    assert!(root.join("anchor").join("metadata.sqlite").exists());
    assert!(root.join("anchor").join("snapshots").is_dir());
    assert!(root.join("anchor").join("cas").is_dir());

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
    assert!(
        negotiate["result"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "environment.status")
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
fn foreground_writes_structured_rpc_logs_with_ids() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-log-rpc-test");

    let health = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "log-health",
            "method": "agent.health"
        }),
    );
    assert_eq!(health["result"]["status"], "ok");

    let log_path = running.root.join("logs").join("agent.log");
    let raw = std::fs::read_to_string(&log_path).unwrap();
    let lines = raw
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    let request_log = lines
        .iter()
        .find(|line| {
            line["message"] == "RPC request received" && line["fields"]["method"] == "agent.health"
        })
        .expect("request log should be present");
    assert_eq!(request_log["request_id"], "log-health");
    assert!(
        request_log["operation_id"]
            .as_str()
            .unwrap()
            .starts_with("op-")
    );

    let response_log = lines
        .iter()
        .find(|line| {
            line["message"] == "RPC response sent" && line["fields"]["method"] == "agent.health"
        })
        .expect("response log should be present");
    assert_eq!(response_log["request_id"], "log-health");
    assert_eq!(response_log["operation_id"], request_log["operation_id"]);
    assert_eq!(response_log["fields"]["status"], "ok");

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
    let db =
        devrelay_core::MetadataDb::open(running.root.join("projects/87654321/metadata.sqlite"))
            .unwrap();
    let sessions = db.list_sessions(Some("87654321")).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "Demo Project");
    assert_eq!(sessions[0].state, devrelay_core::SessionState::Active);

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
fn foreground_serves_environment_status_rpc() {
    use devrelay_core::{
        DevRelayHome, HydrationState, HydrationStateRecord, IpcLimits, UnixIpcConnection,
        save_hydration_state,
    };
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-env-status-rpc-test");
    let repo = running.root.join("env-project");
    create_manifest_repo(&repo, "86421357", "Environment Project");

    let added = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "env-project-add",
            "method": "projects.add",
            "params": {
                "path": repo,
                "manifest": repo.join("devrelay.toml")
            }
        }),
    );
    let workspace_id = added["result"]["project"]["workspaces"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .to_string();

    let cold = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "env-status-cold",
            "method": "environment.status",
            "params": { "project": "86421357" }
        }),
    );
    assert_eq!(cold["result"]["environments"][0]["project_id"], "86421357");
    assert_eq!(
        cold["result"]["environments"][0]["workspace_id"],
        workspace_id
    );
    assert_eq!(cold["result"]["environments"][0]["state"], "cold");
    assert_eq!(cold["result"]["environments"][0]["attempt"], 0);
    assert_eq!(cold["result"]["environments"][0]["persisted"], false);

    let home = DevRelayHome::new(&running.root);
    let mut record = HydrationStateRecord::new("86421357", Some(workspace_id.clone()), 123);
    record.state = HydrationState::ShellReady;
    record.attempt = 2;
    save_hydration_state(
        &home.hydration_state_path("86421357", Some(&workspace_id)),
        &record,
    )
    .unwrap();

    let ready = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "env-status-ready",
            "method": "environment.status",
            "params": {
                "project": "86421357",
                "workspace": workspace_id
            }
        }),
    );
    assert_eq!(ready["result"]["environments"][0]["state"], "shell-ready");
    assert_eq!(ready["result"]["environments"][0]["attempt"], 2);
    assert_eq!(
        ready["result"]["environments"][0]["updated_at_unix_seconds"],
        123
    );
    assert_eq!(ready["result"]["environments"][0]["persisted"], true);

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_restart_preserves_project_and_snapshot_state() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-restart-test");
    let root = running.root.clone();
    let repo = running.root.join("restart-project");
    create_manifest_repo(&repo, "12121212", "Restart Project");

    let added = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "restart-project-add",
            "method": "projects.add",
            "params": {
                "path": repo,
                "manifest": repo.join("devrelay.toml")
            }
        }),
    );
    assert_eq!(added["result"]["project"]["project_id"], "12121212");

    std::fs::write(repo.join("README.md"), "base\nrestart\n").unwrap();
    let checkpoint = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "restart-checkpoint",
            "method": "checkpoint.create",
            "params": {
                "repo": repo,
                "manifest": repo.join("devrelay.toml"),
                "label": "before restart"
            }
        }),
    );
    let snapshot_id = checkpoint["result"]["checkpoint"]["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();

    running.stop_process();
    let mut restarted = RunningAgent::start_existing(root);

    let shown = rpc_call(
        &mut UnixIpcConnection::connect(&restarted.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "restart-project-show",
            "method": "projects.show",
            "params": { "id_or_name": "Restart Project" }
        }),
    );
    assert_eq!(shown["result"]["project"]["project_id"], "12121212");

    let snapshots = rpc_call(
        &mut UnixIpcConnection::connect(&restarted.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "restart-snapshots",
            "method": "snapshots.list",
            "params": { "project": "12121212" }
        }),
    );
    assert!(
        snapshots["result"]["snapshots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|snapshot| snapshot["snapshot_id"] == snapshot_id)
    );

    restarted.stop();
}

#[cfg(unix)]
#[test]
fn foreground_streams_events_and_replays_after_reconnect() {
    use devrelay_core::{
        EventSequence, EventStreamMessage, EventType, IpcConnection, IpcLimits, UnixIpcConnection,
    };
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-events-rpc-test");
    let repo = running.root.join("event-project");
    create_manifest_repo(&repo, "99887766", "Event Project");
    let limits = IpcLimits {
        request_timeout: std::time::Duration::from_secs(5),
        ..IpcLimits::default()
    };

    let mut stream = UnixIpcConnection::connect(&running.socket, limits).unwrap();
    let subscribe = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "events-1",
        "method": "events.subscribe",
        "params": { "cursor": {} }
    }))
    .unwrap();
    stream.write_message(&subscribe, limits).unwrap();
    let ack: serde_json::Value =
        serde_json::from_slice(&stream.read_message(limits).unwrap()).unwrap();
    assert_eq!(ack["id"], "events-1");
    assert_eq!(ack["result"]["replayed"], 0);
    assert!(ack["result"]["current_sequence"].is_null());

    let added = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, limits).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "event-project-add",
            "method": "projects.add",
            "params": {
                "path": repo,
                "manifest": repo.join("devrelay.toml")
            }
        }),
    );
    assert_eq!(added["result"]["project"]["project_id"], "99887766");

    let first = read_stream_message(&mut stream, limits);
    let first_sequence = match first {
        EventStreamMessage::Event { event } => {
            assert_eq!(event.event_type, EventType::WorkspaceStateChanged);
            assert_eq!(event.payload["project_id"], "99887766");
            event.sequence
        }
        EventStreamMessage::Gap { gap } => panic!("unexpected event gap: {gap:?}"),
    };
    drop(stream);

    std::fs::write(repo.join("README.md"), "base\nchanged\n").unwrap();
    let checkpoint = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, limits).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "event-checkpoint",
            "method": "checkpoint.create",
            "params": {
                "repo": repo,
                "manifest": repo.join("devrelay.toml"),
                "label": "event replay"
            }
        }),
    );
    assert_eq!(checkpoint["result"]["checkpoint"]["project_id"], "99887766");

    let mut reconnected = UnixIpcConnection::connect(&running.socket, limits).unwrap();
    let subscribe = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "events-2",
        "method": "events.subscribe",
        "params": {
            "cursor": { "after_sequence": first_sequence.get() }
        }
    }))
    .unwrap();
    reconnected.write_message(&subscribe, limits).unwrap();
    let ack: serde_json::Value =
        serde_json::from_slice(&reconnected.read_message(limits).unwrap()).unwrap();
    assert_eq!(ack["id"], "events-2");
    assert_eq!(ack["result"]["replayed"], 1);

    let replayed = read_stream_message(&mut reconnected, limits);
    match replayed {
        EventStreamMessage::Event { event } => {
            assert_eq!(
                event.sequence,
                EventSequence::new(first_sequence.get() + 1).unwrap()
            );
            assert_eq!(event.event_type, EventType::SnapshotLocalCreated);
            assert_eq!(event.payload["project_id"], "99887766");
            assert_eq!(event.payload["label"], "event replay");
        }
        EventStreamMessage::Gap { gap } => panic!("unexpected event gap: {gap:?}"),
    }

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
fn safety_recovery_defaults_new_workspace_for_agent_rpc() {
    // Invariant: safety/recovery_defaults_new_workspace.
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
    assert_eq!(
        opened["result"]["path"].as_str(),
        Some(recovered.to_str().unwrap())
    );
    assert_ne!(
        opened["result"]["path"].as_str(),
        Some(source.to_str().unwrap())
    );
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
fn safety_diagnostics_redacted_by_default_for_agent_rpc() {
    // Invariant: safety/diagnostics_redacted_by_default.
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;
    use std::io::Write;

    let mut running = RunningAgent::start("devrelay-agent-diagnostics-rpc-test");
    let out = running.root.join("diagnostics").join("bundle.json");
    let repo = running.root.join("diagnostics-project");
    create_manifest_repo(&repo, "24681357", "Diagnostics Project");

    let added = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "diagnostics-project-add",
            "method": "projects.add",
            "params": {
                "path": repo,
                "manifest": repo.join("devrelay.toml")
            }
        }),
    );
    assert_eq!(added["result"]["project"]["project_id"], "24681357");

    let log_path = running.root.join("logs").join("agent.log");
    std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .unwrap();
    let git_exit_log = json!({
        "timestamp_unix_millis": 1234,
        "level": "info",
        "target": "agent.git",
        "message": "Git command completed",
        "request_id": null,
        "operation_id": "op-git",
        "fields": {
            "command": "git",
            "args": format!("-C {} status", running.root.join("private").display()),
            "exit_code": "129",
            "success": "false"
        }
    });
    writeln!(log, "{git_exit_log}").unwrap();

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
    assert!(bundle["capabilities"]["structured_logs"].as_bool().unwrap());
    assert!(bundle["capabilities"]["event_stream"].as_bool().unwrap());
    assert!(bundle["timing"]["duration_millis"].as_u64().is_some());
    assert!(
        !bundle["recent_structured_logs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        bundle["state_machine_records"]["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|session| session["project_id"] == "24681357" && session["state"] == "active")
    );
    assert!(
        bundle["state_machine_records"]["errors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        bundle["git_command_exit_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["command"] == "git"
                && entry["exit_code"] == 129
                && entry["success"] == false
                && entry["args"]
                    .as_str()
                    .is_some_and(|args| args.contains("<path>")))
    );
    let raw_bundle = std::fs::read_to_string(&out).unwrap();
    assert!(
        !raw_bundle.contains(running.root.to_str().unwrap()),
        "diagnostics should redact DEVRELAY_HOME paths by default"
    );
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
#[test]
fn foreground_serves_desktop_bootstrap_rpc_methods() {
    use devrelay_core::{
        AuditEventInput, AuditEventType, AuditOutcome, DeviceIdentity, IpcLimits, MetadataDb,
        UnixIpcConnection,
    };
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-desktop-rpc-test");
    let repo = running.root.join("desktop-project");
    create_manifest_repo(&repo, "13572468", "Desktop Project");

    let added = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "desktop-project-add",
            "method": "projects.add",
            "params": {
                "path": repo,
                "manifest": repo.join("devrelay.toml")
            }
        }),
    );
    assert_eq!(added["result"]["project"]["project_id"], "13572468");

    let global_db = MetadataDb::open(running.root.join("agent.sqlite")).unwrap();
    global_db
        .upsert_device_identity(&DeviceIdentity {
            device_id: "device-desktop".to_string(),
            display_name: "Desktop Device".to_string(),
            platform_key: "test-os".to_string(),
            architecture: "arm64".to_string(),
            capabilities_json: "{}".to_string(),
            paired_at_unix_seconds: Some(10),
            last_seen_unix_seconds: 20,
        })
        .unwrap();
    let mut event = AuditEventInput::new(
        AuditEventType::DevicePaired,
        AuditOutcome::Succeeded,
        "desktop bootstrap event",
    );
    event.project_id = Some("13572468".to_string());
    global_db.record_audit_event(event).unwrap();

    let project_db =
        MetadataDb::open(running.root.join("projects/13572468/metadata.sqlite")).unwrap();
    project_db
        .connection()
        .execute(
            r#"
INSERT INTO task_runs (
    task_run_id,
    project_id,
    session_id,
    state,
    command,
    metadata_json,
    created_at_unix_seconds,
    updated_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
"#,
            (
                "run-desktop",
                "13572468",
                Option::<String>::None,
                "succeeded",
                "cargo test",
                r#"{"source":"test"}"#,
                30_i64,
                40_i64,
            ),
        )
        .unwrap();

    let negotiate = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "desktop-negotiate",
            "method": "rpc.negotiate",
            "params": { "client_protocol_version": 1 }
        }),
    );
    for method in [
        "devices.list",
        "activity.list",
        "runs.list",
        "editor.context.update",
        "settings.get",
        "settings.update",
        "handoffs.list",
        "handoff.begin",
    ] {
        assert!(
            negotiate["result"]["methods"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == method),
            "missing method {method}"
        );
    }

    let devices = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "desktop-devices",
            "method": "devices.list"
        }),
    );
    assert_eq!(
        devices["result"]["devices"][0]["device_id"],
        "device-desktop"
    );

    let activity = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "desktop-activity",
            "method": "activity.list",
            "params": { "project": "13572468", "limit": 10 }
        }),
    );
    assert!(
        activity["result"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["summary"] == "desktop bootstrap event")
    );

    let runs = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "desktop-runs",
            "method": "runs.list",
            "params": { "project": "13572468", "limit": 10 }
        }),
    );
    assert_eq!(runs["result"]["runs"][0]["task_run_id"], "run-desktop");
    assert_eq!(runs["result"]["runs"][0]["metadata"]["source"], "test");

    let settings = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "desktop-settings",
            "method": "settings.get"
        }),
    );
    assert_eq!(settings["result"]["project_count"], 1);
    assert_eq!(settings["result"]["resource_profile"], "balanced");

    let updated = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "desktop-settings-update",
            "method": "settings.update",
            "params": {
                "resource_profile": "eco",
                "mdns_enabled": false,
                "editor_command": "system"
            }
        }),
    );
    assert_eq!(updated["result"]["settings"]["resource_profile"], "eco");
    assert_eq!(updated["result"]["settings"]["mdns_enabled"], false);
    assert_eq!(updated["result"]["settings"]["editor_command"], "system");

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_records_editor_context_update_rpc() {
    use devrelay_core::{IpcLimits, UnixIpcConnection};
    use serde_json::json;

    let mut running = RunningAgent::start("dr-agent-ctx");
    let workspace = running.root.join("editor-project");
    let workspace_path = workspace.to_string_lossy().to_string();
    let active_file = workspace.join("src/main.rs").to_string_lossy().to_string();

    let updated = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "editor-context-update",
            "method": "editor.context.update",
            "params": {
                "project": null,
                "workspace_path": workspace_path,
                "capsule": {
                    "schema_version": 1,
                    "source": "vscode",
                    "workspace": {
                        "folders": [
                            { "name": "editor-project", "path": workspace_path }
                        ]
                    },
                    "tabs": [
                        {
                            "label": "main.rs",
                            "resources": [
                                { "scheme": "file", "path": active_file }
                            ]
                        }
                    ]
                }
            }
        }),
    );
    assert_eq!(updated["result"]["accepted"], true);
    assert!(updated["result"]["capsule_bytes"].as_u64().unwrap() > 0);
    let context_audit_id = updated["result"]["audit_id"].as_i64().unwrap();

    let latest = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "editor-context-latest",
            "method": "editor.context.latest",
            "params": { "project": null }
        }),
    );
    assert_eq!(latest["result"]["context"]["audit_id"], context_audit_id);
    assert_eq!(latest["result"]["context"]["capsule"]["source"], "vscode");

    let ack = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "editor-restore-ack",
            "method": "editor.restore.ack",
            "params": {
                "project": null,
                "restored_context_audit_id": context_audit_id,
                "succeeded": true,
                "partial": false,
                "detail": {
                    "opened_files": [active_file],
                    "partial_details": []
                }
            }
        }),
    );
    assert_eq!(ack["result"]["accepted"], true);

    let activity = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "editor-context-activity",
            "method": "activity.list",
            "params": { "limit": 10 }
        }),
    );
    let events = activity["result"]["events"].as_array().unwrap();
    let event = events
        .iter()
        .find(|event| event["type"] == "editor.context.updated")
        .unwrap();
    assert_eq!(event["type"], "editor.context.updated");
    assert_eq!(event["summary"], "editor context updated");
    assert_eq!(event["detail"]["capsule"]["source"], "vscode");
    assert_eq!(
        event["detail"]["workspace_path"].as_str(),
        Some(workspace_path.as_str())
    );
    assert!(events.iter().any(|event| {
        event["type"] == "editor.restore.acked"
            && event["detail"]["restored_context_audit_id"] == context_audit_id
    }));

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_editor_event_aborts_source_handoff() {
    use devrelay_core::{
        DeviceIdentity, IpcConnection, IpcLimits, LeaseRecord, LeaseState, MetadataDb,
        UnixIpcConnection,
    };
    use serde_json::json;

    let mut running = RunningAgent::start("dr-agent-editguard");
    let repo = running.root.join("editguard-project");
    create_manifest_repo(&repo, "97531864", "Edit Guard Project");

    let added = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "editguard-project-add",
            "method": "projects.add",
            "params": {
                "path": repo,
                "manifest": repo.join("devrelay.toml")
            }
        }),
    );
    assert_eq!(added["result"]["project"]["project_id"], "97531864");

    let settings = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "editguard-settings",
            "method": "settings.get"
        }),
    );
    let source_device_id = settings["result"]["device_id"]
        .as_str()
        .unwrap()
        .to_string();
    let global_db = MetadataDb::open(running.root.join("agent.sqlite")).unwrap();
    global_db
        .upsert_device_identity(&DeviceIdentity {
            device_id: "device-editguard-target".to_string(),
            display_name: "Edit Guard Target".to_string(),
            platform_key: "linux-gnu-x86_64".to_string(),
            architecture: "x86_64".to_string(),
            capabilities_json: "{}".to_string(),
            paired_at_unix_seconds: Some(100),
            last_seen_unix_seconds: 200,
        })
        .unwrap();

    let project_db =
        MetadataDb::open(running.root.join("projects/97531864/metadata.sqlite")).unwrap();
    let session = project_db
        .list_sessions(Some("97531864"))
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    project_db
        .upsert_lease(&LeaseRecord {
            lease_id: "lease-editguard".to_string(),
            project_id: "97531864".to_string(),
            session_id: session.session_id,
            state: LeaseState::Active,
            epoch: 3,
            holder_device_id: Some(source_device_id.clone()),
            latest_snapshot_id: None,
            handoff_id: None,
        })
        .unwrap();

    let begin = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "editguard-handoff-begin",
            "method": "handoff.begin",
            "params": {
                "project": "97531864",
                "lease_id": "lease-editguard",
                "target_device_id": "device-editguard-target",
                "source_generation": "editor-0",
                "ttl_seconds": 300
            }
        }),
    );
    let handoff_id = begin["result"]["handoff"]["handoff_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(begin["result"]["handoff"]["state"], "target-prepare");

    let mut events = UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap();
    let subscribe = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "editguard-events",
        "method": "events.subscribe",
        "params": { "cursor": {} }
    }))
    .unwrap();
    events
        .write_message(&subscribe, IpcLimits::default())
        .unwrap();
    let ack: serde_json::Value =
        serde_json::from_slice(&events.read_message(IpcLimits::default()).unwrap()).unwrap();
    for _ in 0..ack["result"]["replayed"].as_u64().unwrap_or_default() {
        let _ = read_stream_message(&mut events, IpcLimits::default());
    }
    let workspace_path = repo.to_string_lossy().to_string();
    let document_path = repo.join("src/main.rs").to_string_lossy().to_string();
    let document_uri = format!("file://{document_path}");

    let edit = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "editguard-editor-event",
            "method": "editor.event.record",
            "params": {
                "project": null,
                "workspace_path": workspace_path,
                "event_kind": "text-document-changed",
                "document_uri": document_uri,
                "document_path": document_path,
                "document_version": 2,
                "meaningful_edit": true
            }
        }),
    );
    assert_eq!(edit["result"]["project"], "97531864");
    assert_eq!(edit["result"]["source_generation"], 1);
    assert_eq!(
        edit["result"]["aborted_handoffs"][0]["handoff_id"],
        handoff_id
    );
    assert_eq!(edit["result"]["aborted_handoffs"][0]["state"], "aborted");
    assert_handoff_state_event(
        &mut events,
        IpcLimits::default(),
        &handoff_id,
        "aborted",
        Some("target-prepare"),
    );

    let lease = project_db.get_lease("lease-editguard").unwrap().unwrap();
    assert_eq!(lease.state, LeaseState::Active);
    assert_eq!(
        lease.holder_device_id.as_deref(),
        Some(source_device_id.as_str())
    );
    assert_eq!(lease.handoff_id, None);

    running.stop();
}

#[cfg(unix)]
#[test]
fn foreground_serves_handoff_state_machine_rpc_methods() {
    use devrelay_core::{
        DeviceIdentity, IpcConnection, IpcLimits, LeaseRecord, LeaseState, MetadataDb,
        UnixIpcConnection,
    };
    use serde_json::json;

    let mut running = RunningAgent::start("devrelay-agent-handoff-rpc-test");
    let repo = running.root.join("handoff-project");
    create_manifest_repo(&repo, "24681357", "Handoff Project");

    let added = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "handoff-project-add",
            "method": "projects.add",
            "params": {
                "path": repo,
                "manifest": repo.join("devrelay.toml")
            }
        }),
    );
    assert_eq!(added["result"]["project"]["project_id"], "24681357");

    let settings = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "handoff-settings",
            "method": "settings.get"
        }),
    );
    let source_device_id = settings["result"]["device_id"]
        .as_str()
        .unwrap()
        .to_string();
    let global_db = MetadataDb::open(running.root.join("agent.sqlite")).unwrap();
    global_db
        .upsert_device_identity(&DeviceIdentity {
            device_id: "device-target".to_string(),
            display_name: "Target Device".to_string(),
            platform_key: "linux-gnu-x86_64".to_string(),
            architecture: "x86_64".to_string(),
            capabilities_json: "{}".to_string(),
            paired_at_unix_seconds: Some(100),
            last_seen_unix_seconds: 200,
        })
        .unwrap();

    let project_db =
        MetadataDb::open(running.root.join("projects/24681357/metadata.sqlite")).unwrap();
    let session = project_db
        .list_sessions(Some("24681357"))
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    project_db
        .upsert_lease(&LeaseRecord {
            lease_id: "lease-handoff-rpc".to_string(),
            project_id: "24681357".to_string(),
            session_id: session.session_id,
            state: LeaseState::Active,
            epoch: 7,
            holder_device_id: Some(source_device_id.clone()),
            latest_snapshot_id: None,
            handoff_id: None,
        })
        .unwrap();

    let negotiate = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "handoff-negotiate",
            "method": "rpc.negotiate",
            "params": { "client_protocol_version": 1 }
        }),
    );
    for method in [
        "handoffs.list",
        "leases.list",
        "handoff.begin",
        "handoff.target.verify",
        "handoff.source.ready",
        "handoff.commit",
        "handoff.abort",
        "handoff.recover",
    ] {
        assert!(
            negotiate["result"]["methods"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == method),
            "missing method {method}"
        );
    }

    let leases = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "leases-list",
            "method": "leases.list",
            "params": { "project": "24681357" }
        }),
    );
    assert_eq!(
        leases["result"]["leases"][0]["lease_id"],
        "lease-handoff-rpc"
    );
    assert_eq!(
        leases["result"]["leases"][0]["holder_device_id"],
        source_device_id
    );

    let limits = IpcLimits::default();
    let mut handoff_events = UnixIpcConnection::connect(&running.socket, limits).unwrap();
    let subscribe = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "handoff-events",
        "method": "events.subscribe",
        "params": { "cursor": {} }
    }))
    .unwrap();
    handoff_events.write_message(&subscribe, limits).unwrap();
    let ack: serde_json::Value =
        serde_json::from_slice(&handoff_events.read_message(limits).unwrap()).unwrap();
    assert_eq!(ack["id"], "handoff-events");
    for _ in 0..ack["result"]["replayed"].as_u64().unwrap_or_default() {
        let _ = read_stream_message(&mut handoff_events, limits);
    }

    let begin = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "handoff-begin",
            "method": "handoff.begin",
            "params": {
                "project": "24681357",
                "lease_id": "lease-handoff-rpc",
                "target_device_id": "device-target",
                "source_generation": "gen-rpc",
                "ttl_seconds": 300
            }
        }),
    );
    assert_eq!(begin["result"]["handoff"]["state"], "target-prepare");
    assert_eq!(
        begin["result"]["handoff"]["source_device_id"],
        source_device_id
    );
    let handoff_id = begin["result"]["handoff"]["handoff_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_handoff_state_event(
        &mut handoff_events,
        limits,
        &handoff_id,
        "target-prepare",
        None,
    );
    assert!(
        begin["result"]["journal"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["phase"] == "begin")
    );

    let listed = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "handoffs-list",
            "method": "handoffs.list",
            "params": { "project": "24681357" }
        }),
    );
    assert_eq!(
        listed["result"]["handoffs"][0]["record"]["handoff_id"],
        handoff_id
    );
    assert!(
        listed["result"]["handoffs"][0]["journal"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["phase"] == "target-prepare")
    );

    let target_verified = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "handoff-target-verify",
            "method": "handoff.target.verify",
            "params": {
                "project": "24681357",
                "handoff_id": handoff_id
            }
        }),
    );
    assert_eq!(
        target_verified["result"]["handoff"]["state"],
        "target-verified"
    );
    assert_handoff_state_event(
        &mut handoff_events,
        limits,
        &handoff_id,
        "target-verified",
        Some("target-prepare"),
    );

    let source_ready = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "handoff-source-ready",
            "method": "handoff.source.ready",
            "params": {
                "project": "24681357",
                "handoff_id": handoff_id
            }
        }),
    );
    assert_eq!(source_ready["result"]["handoff"]["state"], "source-ready");
    assert_handoff_state_event(
        &mut handoff_events,
        limits,
        &handoff_id,
        "source-ready",
        Some("target-verified"),
    );

    let committed = rpc_call(
        &mut UnixIpcConnection::connect(&running.socket, IpcLimits::default()).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": "handoff-commit",
            "method": "handoff.commit",
            "params": {
                "project": "24681357",
                "handoff_id": handoff_id,
                "observed_source_generation": "gen-rpc"
            }
        }),
    );
    assert_eq!(committed["result"]["handoff"]["state"], "committed");
    assert_handoff_state_event(
        &mut handoff_events,
        limits,
        &handoff_id,
        "committed",
        Some("source-ready"),
    );

    let lease = project_db.get_lease("lease-handoff-rpc").unwrap().unwrap();
    assert_eq!(lease.holder_device_id.as_deref(), Some("device-target"));
    assert_eq!(lease.epoch, 8);
    assert_eq!(lease.handoff_id, None);

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
fn read_stream_message(
    connection: &mut devrelay_core::UnixIpcConnection,
    limits: devrelay_core::IpcLimits,
) -> devrelay_core::EventStreamMessage {
    use devrelay_core::IpcConnection;

    let response = connection.read_message(limits).unwrap();
    serde_json::from_slice(&response).unwrap()
}

#[cfg(unix)]
fn assert_handoff_state_event(
    connection: &mut devrelay_core::UnixIpcConnection,
    limits: devrelay_core::IpcLimits,
    handoff_id: &str,
    state: &str,
    previous_state: Option<&str>,
) {
    match read_stream_message(connection, limits) {
        devrelay_core::EventStreamMessage::Event { event } => {
            assert_eq!(
                event.event_type,
                devrelay_core::EventType::HandoffStateChanged
            );
            assert_eq!(event.payload["handoff_id"], handoff_id);
            assert_eq!(event.payload["state"], state);
            if let Some(previous_state) = previous_state {
                assert_eq!(event.payload["previous_state"], previous_state);
            } else {
                assert!(event.payload["previous_state"].is_null());
            }
            assert!(event.payload.get("source_generation").is_none());
        }
        devrelay_core::EventStreamMessage::Gap { gap } => {
            panic!("unexpected event gap: {gap:?}");
        }
    }
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
        let root = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        Self::start_at(root, true)
    }

    fn start_existing(root: std::path::PathBuf) -> Self {
        Self::start_at(root, false)
    }

    fn start_at(root: std::path::PathBuf, reset: bool) -> Self {
        use std::os::unix::fs::FileTypeExt;
        use std::process::Stdio;
        use std::time::{Duration, Instant};

        let config = root.join("config.toml");
        let socket = root.join("agent.sock");
        if reset {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root).unwrap();
        let _ = std::fs::remove_file(&socket);

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

    fn stop_process(&mut self) {
        self.child.kill().ok();
        let _ = self.child.wait();
    }

    fn stop(&mut self) {
        self.stop_process();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
impl Drop for RunningAgent {
    fn drop(&mut self) {
        self.stop();
    }
}
