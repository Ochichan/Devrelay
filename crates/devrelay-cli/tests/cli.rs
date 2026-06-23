use std::path::Path;
use std::process::Command;

fn devrelay() -> Command {
    Command::new(env!("CARGO_BIN_EXE_devrelay"))
}

fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}

fn init_git_repo(root: &Path) {
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "DevRelay Test"]);
    git(
        root,
        &["config", "user.email", "devrelay-test@example.local"],
    );
    std::fs::write(root.join("README.md"), "demo\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "base"]);
}

fn write_manifest(root: &Path, project_id: &str, name: &str) {
    std::fs::write(
        root.join("devrelay.toml"),
        format!(
            r#"schema = 1
project_id = "{project_id}"
name = "{name}"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#
        ),
    )
    .unwrap();
}

fn workspace_state_for(project: &serde_json::Value, path: &Path) -> Option<String> {
    let canonical = path.canonicalize().unwrap();
    project["workspaces"]
        .as_object()
        .unwrap()
        .values()
        .find(|workspace| workspace["local_path"].as_str() == canonical.to_str())
        .and_then(|workspace| workspace["state"].as_str())
        .map(str::to_string)
}

#[test]
fn prints_version() {
    let output = devrelay().arg("--version").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("devrelay"));
}

#[test]
fn exposes_agent_routing_global_flags() {
    let output = devrelay().arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--direct"));
    assert!(stdout.contains("--agent-socket"));
}

#[cfg(unix)]
#[test]
fn status_uses_agent_rpc_by_default() {
    use devrelay_core::{
        IpcConnection, IpcLimits, IpcTransport, METHOD_STATUS_GET, RpcRequest, RpcResponse,
        UnixIpcListener,
    };
    use serde_json::json;

    let root = std::env::temp_dir().join(format!(
        "devrelay-status-agent-test-{}-{}",
        std::process::id(),
        "repo"
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);
    write_manifest(&root, "agent-status-project", "Agent Status Project");
    git(&root, &["add", "devrelay.toml"]);
    git(&root, &["commit", "-m", "manifest"]);

    let manifest = root.join("devrelay.toml");
    let socket = root.join("agent.sock");
    let listener = UnixIpcListener::bind(&socket).unwrap();
    let expected_repo = root.clone();
    let expected_manifest = manifest.clone();
    let handle = std::thread::spawn(move || {
        let mut connection = listener.accept().unwrap();
        let request_bytes = connection.read_message(IpcLimits::default()).unwrap();
        let request = RpcRequest::parse(&request_bytes).unwrap();
        assert_eq!(request.method, METHOD_STATUS_GET);
        assert_eq!(
            request.params["repo"].as_str(),
            Some(expected_repo.to_str().unwrap())
        );
        assert_eq!(
            request.params["manifest"].as_str(),
            Some(expected_manifest.to_str().unwrap())
        );
        let response = RpcResponse::success(
            request.required_id().unwrap(),
            json!({
                "status": {
                    "head_oid": "agent-head",
                    "branch": "agent-branch",
                    "upstream": null,
                    "counts": {
                        "staged": 0,
                        "unstaged": 0,
                        "untracked": 1,
                        "ignored": 0,
                        "unmerged": 0
                    },
                    "clean": false,
                    "initial": false
                },
                "entries": [],
                "untracked": [
                    {
                        "path": "agent-only.txt",
                        "decision": "include",
                        "reason": "safe-untracked"
                    }
                ]
            }),
        );
        connection
            .write_message(
                &serde_json::to_vec(&response).unwrap(),
                IpcLimits::default(),
            )
            .unwrap();
    });

    let output = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "status",
            "--repo",
            root.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout["project"].as_str(), Some("Agent Status Project"));
    assert_eq!(stdout["status"]["branch"].as_str(), Some("agent-branch"));
    assert_eq!(
        stdout["untracked_policy"][0]["path"].as_str(),
        Some("agent-only.txt")
    );
    handle.join().unwrap();

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn status_direct_bypasses_agent_socket() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-status-direct-test-{}-{}",
        std::process::id(),
        "repo"
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);
    write_manifest(&root, "direct-status-project", "Direct Status Project");
    git(&root, &["add", "devrelay.toml"]);
    git(&root, &["commit", "-m", "manifest"]);

    let manifest = root.join("devrelay.toml");
    let output = devrelay()
        .args([
            "--direct",
            "--agent-socket",
            root.join("missing.sock").to_str().unwrap(),
            "status",
            "--repo",
            root.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout["project"].as_str(), Some("Direct Status Project"));
    assert_eq!(stdout["status"]["branch"].as_str(), Some("main"));

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn project_commands_use_agent_rpc_by_default() {
    use devrelay_core::{
        IpcConnection, IpcLimits, IpcTransport, METHOD_PROJECTS_ADD, METHOD_PROJECTS_LIST,
        METHOD_PROJECTS_REMOVE, METHOD_PROJECTS_SHOW, RpcRequest, RpcResponse, UnixIpcListener,
    };
    use serde_json::json;

    let root = std::env::temp_dir().join(format!(
        "devrelay-project-agent-test-{}-{}",
        std::process::id(),
        "repo"
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);
    write_manifest(&root, "agent-project", "Agent Project");
    git(&root, &["add", "devrelay.toml"]);
    git(&root, &["commit", "-m", "manifest"]);

    let manifest = root.join("devrelay.toml");
    let socket = root.join("agent.sock");
    let listener = UnixIpcListener::bind(&socket).unwrap();
    let expected_repo = root.to_str().unwrap().to_string();
    let expected_manifest = manifest.to_str().unwrap().to_string();
    let project = json!({
        "project_id": "agent-project",
        "display_name": "Agent Project",
        "local_path": expected_repo,
        "workspaces": {
            "w_agent": {
                "workspace_id": "w_agent",
                "project_id": "agent-project",
                "device_id": "agent-device",
                "local_path": expected_repo,
                "platform_profile": "macos-aarch64",
                "state": "active",
                "last_seen_head": "agent-head",
                "last_checkpoint_id": null
            }
        },
        "manifest_path": expected_manifest,
        "remote_url_fingerprint": null,
        "root_commit_fingerprint": "root_agent"
    });
    let handle = std::thread::spawn(move || {
        for method in [
            METHOD_PROJECTS_ADD,
            METHOD_PROJECTS_LIST,
            METHOD_PROJECTS_SHOW,
            METHOD_PROJECTS_REMOVE,
        ] {
            let mut connection = listener.accept().unwrap();
            let request_bytes = connection.read_message(IpcLimits::default()).unwrap();
            let request = RpcRequest::parse(&request_bytes).unwrap();
            assert_eq!(request.method, method);

            let result = match method {
                METHOD_PROJECTS_ADD => {
                    assert_eq!(
                        request.params["path"].as_str(),
                        project["local_path"].as_str()
                    );
                    assert_eq!(
                        request.params["manifest"].as_str(),
                        project["manifest_path"].as_str()
                    );
                    json!({ "project": project.clone() })
                }
                METHOD_PROJECTS_LIST => json!({ "projects": [project.clone()] }),
                METHOD_PROJECTS_SHOW => {
                    assert_eq!(request.params["id_or_name"].as_str(), Some("Agent Project"));
                    json!({ "project": project.clone() })
                }
                METHOD_PROJECTS_REMOVE => {
                    assert_eq!(request.params["id_or_name"].as_str(), Some("Agent Project"));
                    json!({ "project": project.clone() })
                }
                _ => unreachable!(),
            };
            let response = RpcResponse::success(request.required_id().unwrap(), result);
            connection
                .write_message(
                    &serde_json::to_vec(&response).unwrap(),
                    IpcLimits::default(),
                )
                .unwrap();
        }
    });

    let add = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "project",
            "add",
            root.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );
    let add_json: serde_json::Value = serde_json::from_slice(&add.stdout).unwrap();
    assert_eq!(
        add_json["added"]["project_id"].as_str(),
        Some("agent-project")
    );
    assert!(add_json["config"].as_str().is_some());

    let list = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "projects",
            "list",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list_json: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(list_json[0]["display_name"].as_str(), Some("Agent Project"));

    let show = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "project",
            "show",
            "Agent Project",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&show.stderr)
    );
    let show_json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(show_json["project_id"].as_str(), Some("agent-project"));

    let remove = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "project",
            "remove",
            "Agent Project",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        remove.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&remove.stderr)
    );
    let remove_json: serde_json::Value = serde_json::from_slice(&remove.stdout).unwrap();
    assert_eq!(
        remove_json["removed"]["project_id"].as_str(),
        Some("agent-project")
    );
    handle.join().unwrap();

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn checkpoint_uses_agent_rpc_by_default_and_writes_out_file() {
    use devrelay_core::{
        IpcConnection, IpcLimits, IpcTransport, METHOD_CHECKPOINT_CREATE, RpcRequest, RpcResponse,
        UnixIpcListener,
    };
    use serde_json::json;

    let root = std::env::temp_dir().join(format!(
        "devrelay-checkpoint-agent-test-{}-{}",
        std::process::id(),
        "repo"
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);
    write_manifest(
        &root,
        "agent-checkpoint-project",
        "Agent Checkpoint Project",
    );
    git(&root, &["add", "devrelay.toml"]);
    git(&root, &["commit", "-m", "manifest"]);

    let manifest = root.join("devrelay.toml");
    let socket = root.join("agent.sock");
    let out = root.join("agent-snapshot.json");
    let snapshot_repo = root.join("snapshots.git");
    let metadata = json!({
        "schema_version": 1,
        "snapshot_id": "s1_0123456789abcdef01234567",
        "project_id": "agent-checkpoint-project",
        "project_name": "Agent Checkpoint Project",
        "session_id": null,
        "parent_snapshot_id": null,
        "source_device_id": null,
        "branch": "main",
        "head_oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "index_tree_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "index_commit_oid": "cccccccccccccccccccccccccccccccccccccccc",
        "work_tree_oid": "dddddddddddddddddddddddddddddddddddddddd",
        "work_commit_oid": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "source_status": {
            "staged": 0,
            "unstaged": 1,
            "untracked": 1,
            "ignored": 0,
            "unmerged": 0
        },
        "included_untracked": ["notes.md"],
        "excluded": [],
        "state_hash": "agent-state-hash",
        "created_at_unix_seconds": 1234567890_u64
    });
    let checkpoint = json!({
        "snapshot_id": "s1_0123456789abcdef01234567",
        "project_id": "agent-checkpoint-project",
        "session_id": null,
        "parent_snapshot_id": null,
        "sequence_number": 7,
        "pinned": true,
        "label": "agent checkpoint",
        "metadata": metadata,
        "created_at_unix_seconds": 1234567890_u64
    });

    let listener = UnixIpcListener::bind(&socket).unwrap();
    let expected_repo = root.to_str().unwrap().to_string();
    let expected_manifest = manifest.to_str().unwrap().to_string();
    let expected_snapshot_repo = snapshot_repo.to_str().unwrap().to_string();
    let handle = std::thread::spawn(move || {
        let mut connection = listener.accept().unwrap();
        let request_bytes = connection.read_message(IpcLimits::default()).unwrap();
        let request = RpcRequest::parse(&request_bytes).unwrap();
        assert_eq!(request.method, METHOD_CHECKPOINT_CREATE);
        assert_eq!(
            request.params["repo"].as_str(),
            Some(expected_repo.as_str())
        );
        assert_eq!(
            request.params["manifest"].as_str(),
            Some(expected_manifest.as_str())
        );
        assert_eq!(request.params["label"].as_str(), Some("agent checkpoint"));
        assert_eq!(request.params["pin"].as_bool(), Some(true));
        let response = RpcResponse::success(
            request.required_id().unwrap(),
            json!({
                "checkpoint": checkpoint,
                "snapshot_repo": expected_snapshot_repo
            }),
        );
        connection
            .write_message(
                &serde_json::to_vec(&response).unwrap(),
                IpcLimits::default(),
            )
            .unwrap();
    });

    let output = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "checkpoint",
            "--repo",
            root.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--label",
            "agent checkpoint",
            "--pin",
            "--out",
            out.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        stdout["checkpoint"]["snapshot_id"].as_str(),
        Some("s1_0123456789abcdef01234567")
    );
    assert_eq!(stdout["checkpoint"]["sequence_number"].as_i64(), Some(7));
    assert_eq!(
        stdout["snapshot_file"].as_str(),
        Some(out.to_str().unwrap())
    );
    assert_eq!(
        stdout["snapshot_repo"].as_str(),
        Some(snapshot_repo.to_str().unwrap())
    );
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(
        written["snapshot_id"].as_str(),
        Some("s1_0123456789abcdef01234567")
    );
    handle.join().unwrap();

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn recover_commands_use_agent_rpc_by_default() {
    use devrelay_core::{
        IpcConnection, IpcLimits, IpcTransport, METHOD_RECOVER_LIST, METHOD_RECOVER_OPEN,
        METHOD_RECOVER_SHOW, RpcRequest, RpcResponse, UnixIpcListener,
    };
    use serde_json::json;

    let root = std::env::temp_dir().join(format!(
        "devrelay-recover-agent-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let socket = root.join("agent.sock");
    let recovered = root.join("recovered");
    let snapshot_id = "s1_111111111111111111111111";
    let metadata = json!({
        "schema_version": 1,
        "snapshot_id": snapshot_id,
        "project_id": "agent-recover-project",
        "project_name": "Agent Recover Project",
        "session_id": null,
        "parent_snapshot_id": null,
        "source_device_id": null,
        "branch": "main",
        "head_oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "index_tree_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "index_commit_oid": "cccccccccccccccccccccccccccccccccccccccc",
        "work_tree_oid": "dddddddddddddddddddddddddddddddddddddddd",
        "work_commit_oid": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "source_status": {
            "staged": 0,
            "unstaged": 0,
            "untracked": 1,
            "ignored": 0,
            "unmerged": 0
        },
        "included_untracked": ["notes.md"],
        "excluded": [],
        "state_hash": "recover-state-hash",
        "created_at_unix_seconds": 1234567890_u64
    });
    let snapshot = json!({
        "snapshot_id": snapshot_id,
        "project_id": "agent-recover-project",
        "session_id": null,
        "parent_snapshot_id": null,
        "sequence_number": 3,
        "pinned": false,
        "label": "recoverable",
        "metadata": metadata,
        "created_at_unix_seconds": 1234567890_u64
    });
    let verification = json!({
        "head_oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "index_tree_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "work_tree_oid": "dddddddddddddddddddddddddddddddddddddddd",
        "state_hash": "recover-state-hash",
        "included_untracked": ["notes.md"],
        "excluded_paths": []
    });

    let listener = UnixIpcListener::bind(&socket).unwrap();
    let expected_recovered = recovered.to_str().unwrap().to_string();
    let handle = std::thread::spawn(move || {
        for method in [
            METHOD_RECOVER_LIST,
            METHOD_RECOVER_SHOW,
            METHOD_RECOVER_OPEN,
        ] {
            let mut connection = listener.accept().unwrap();
            let request_bytes = connection.read_message(IpcLimits::default()).unwrap();
            let request = RpcRequest::parse(&request_bytes).unwrap();
            assert_eq!(request.method, method);
            let result = match method {
                METHOD_RECOVER_LIST => {
                    assert_eq!(
                        request.params["project"].as_str(),
                        Some("agent-recover-project")
                    );
                    json!({ "snapshots": [snapshot.clone()] })
                }
                METHOD_RECOVER_SHOW => {
                    assert_eq!(request.params["snapshot_id"].as_str(), Some(snapshot_id));
                    assert_eq!(
                        request.params["project"].as_str(),
                        Some("agent-recover-project")
                    );
                    json!({ "snapshot": snapshot.clone() })
                }
                METHOD_RECOVER_OPEN => {
                    assert_eq!(request.params["snapshot_id"].as_str(), Some(snapshot_id));
                    assert_eq!(
                        request.params["path"].as_str(),
                        Some(expected_recovered.as_str())
                    );
                    assert_eq!(request.params["register"].as_bool(), Some(true));
                    assert_eq!(request.params["name"].as_str(), Some("Recovered copy"));
                    json!({
                        "recovered": snapshot.clone(),
                        "path": expected_recovered,
                        "name": "Recovered copy",
                        "registered": null,
                        "verification": verification
                    })
                }
                _ => unreachable!(),
            };
            let response = RpcResponse::success(request.required_id().unwrap(), result);
            connection
                .write_message(
                    &serde_json::to_vec(&response).unwrap(),
                    IpcLimits::default(),
                )
                .unwrap();
        }
    });

    let list = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "recover",
            "list",
            "--project",
            "agent-recover-project",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list_json: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(list_json[0]["snapshot_id"].as_str(), Some(snapshot_id));

    let show = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "recover",
            "show",
            snapshot_id,
            "--project",
            "agent-recover-project",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&show.stderr)
    );
    let show_json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(show_json["snapshot_id"].as_str(), Some(snapshot_id));

    let open = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "recover",
            "open",
            snapshot_id,
            "--project",
            "agent-recover-project",
            "--path",
            recovered.to_str().unwrap(),
            "--register",
            "--name",
            "Recovered copy",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        open.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&open.stderr)
    );
    let open_json: serde_json::Value = serde_json::from_slice(&open.stdout).unwrap();
    assert_eq!(
        open_json["recovered"]["snapshot_id"].as_str(),
        Some(snapshot_id)
    );
    assert_eq!(open_json["name"].as_str(), Some("Recovered copy"));
    assert_eq!(
        open_json["verification"]["included_untracked"][0].as_str(),
        Some("notes.md")
    );
    handle.join().unwrap();

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn apply_uses_agent_rpc_by_default_for_block_policy() {
    use devrelay_core::{
        IpcConnection, IpcLimits, IpcTransport, METHOD_APPLY_SNAPSHOT, RpcRequest, RpcResponse,
        UnixIpcListener,
    };
    use serde_json::json;

    let root =
        std::env::temp_dir().join(format!("devrelay-apply-agent-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let socket = root.join("agent.sock");
    let target = root.join("target");
    let source = root.join("unused-source");
    let snapshot_path = root.join("snapshot.json");
    let snapshot_id = "s1_222222222222222222222222";
    let metadata = json!({
        "schema_version": 1,
        "snapshot_id": snapshot_id,
        "project_id": "agent-apply-project",
        "project_name": "Agent Apply Project",
        "session_id": null,
        "parent_snapshot_id": null,
        "source_device_id": null,
        "branch": "main",
        "head_oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "index_tree_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "index_commit_oid": "cccccccccccccccccccccccccccccccccccccccc",
        "work_tree_oid": "dddddddddddddddddddddddddddddddddddddddd",
        "work_commit_oid": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "source_status": {
            "staged": 0,
            "unstaged": 0,
            "untracked": 1,
            "ignored": 0,
            "unmerged": 0
        },
        "included_untracked": ["notes.md"],
        "excluded": [],
        "state_hash": "apply-state-hash",
        "created_at_unix_seconds": 1234567890_u64
    });
    std::fs::write(
        &snapshot_path,
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    let snapshot = json!({
        "snapshot_id": snapshot_id,
        "project_id": "agent-apply-project",
        "session_id": null,
        "parent_snapshot_id": null,
        "sequence_number": 4,
        "pinned": false,
        "label": "apply me",
        "metadata": metadata,
        "created_at_unix_seconds": 1234567890_u64
    });
    let plan = json!({
        "snapshot_id": snapshot_id,
        "branch": "main",
        "detached": false,
        "head_oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "index_ref": format!("refs/devrelay/snapshots/{snapshot_id}/index"),
        "work_ref": format!("refs/devrelay/snapshots/{snapshot_id}/work")
    });
    let verification = json!({
        "head_oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "index_tree_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "work_tree_oid": "dddddddddddddddddddddddddddddddddddddddd",
        "state_hash": "apply-state-hash",
        "included_untracked": ["notes.md"],
        "excluded_paths": []
    });

    let listener = UnixIpcListener::bind(&socket).unwrap();
    let expected_target = target.to_str().unwrap().to_string();
    let handle = std::thread::spawn(move || {
        for dry_run in [true, false] {
            let mut connection = listener.accept().unwrap();
            let request_bytes = connection.read_message(IpcLimits::default()).unwrap();
            let request = RpcRequest::parse(&request_bytes).unwrap();
            assert_eq!(request.method, METHOD_APPLY_SNAPSHOT);
            assert_eq!(
                request.params["repo"].as_str(),
                Some(expected_target.as_str())
            );
            assert_eq!(
                request.params["project"].as_str(),
                Some("agent-apply-project")
            );
            assert_eq!(request.params["snapshot_id"].as_str(), Some(snapshot_id));
            assert_eq!(request.params["dry_run"].as_bool(), Some(dry_run));
            let result = if dry_run {
                json!({
                    "snapshot": snapshot.clone(),
                    "plan": plan,
                    "verification": null
                })
            } else {
                json!({
                    "snapshot": snapshot.clone(),
                    "plan": null,
                    "verification": verification
                })
            };
            let response = RpcResponse::success(request.required_id().unwrap(), result);
            connection
                .write_message(
                    &serde_json::to_vec(&response).unwrap(),
                    IpcLimits::default(),
                )
                .unwrap();
        }
    });

    let dry_run = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "apply",
            "--repo",
            target.to_str().unwrap(),
            "--source",
            source.to_str().unwrap(),
            "--snapshot",
            snapshot_path.to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        dry_run.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_run_json: serde_json::Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(dry_run_json["dry_run"].as_bool(), Some(true));
    assert_eq!(
        dry_run_json["plan"]["snapshot_id"].as_str(),
        Some(snapshot_id)
    );

    let apply = devrelay()
        .args([
            "--agent-socket",
            socket.to_str().unwrap(),
            "apply",
            "--repo",
            target.to_str().unwrap(),
            "--source",
            source.to_str().unwrap(),
            "--snapshot",
            snapshot_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        apply.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply_json: serde_json::Value = serde_json::from_slice(&apply.stdout).unwrap();
    assert_eq!(apply_json["applied"].as_str(), Some(snapshot_id));
    assert_eq!(apply_json["dirty_policy"].as_str(), Some("block"));
    assert!(apply_json["backup"].is_null());
    assert_eq!(
        apply_json["verification"]["included_untracked"][0].as_str(),
        Some("notes.md")
    );
    handle.join().unwrap();

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn manifest_check_supports_json() {
    let output = devrelay()
        .args([
            "manifest",
            "check",
            "../../devrelay_spec_bundle/devrelay.toml",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"ok\": true"));
    assert!(stdout.contains("\"project_id\""));
}

#[test]
fn json_errors_use_nonzero_exit() {
    let output = devrelay()
        .args([
            "--json-errors",
            "manifest",
            "check",
            "../../missing-devrelay.toml",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("\"error\""));
    assert!(stderr.contains("\"code\""));
    assert!(stderr.contains("\"title\""));
    assert!(stderr.contains("\"detail\""));
    assert!(stderr.contains("\"safe_actions\""));
    assert!(stderr.contains("\"diagnostic_id\""));
}

#[test]
fn config_save_and_load_round_trip() {
    let path = std::env::temp_dir().join(format!(
        "devrelay-config-test-{}-{}.toml",
        std::process::id(),
        "round-trip"
    ));
    let _ = std::fs::remove_file(&path);

    let save = devrelay()
        .args(["config", "save", "--path", path.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(save.status.success());

    let load = devrelay()
        .args(["config", "load", "--path", path.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(load.status.success());
    let stdout = String::from_utf8(load.stdout).unwrap();
    assert!(stdout.contains("\"version\": 1"));
    assert!(stdout.contains("\"device_name\""));

    let _ = std::fs::remove_file(path);
}

#[test]
fn project_registry_commands_round_trip() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-project-test-{}-{}",
        std::process::id(),
        "repo"
    ));
    let config = std::env::temp_dir().join(format!(
        "devrelay-project-test-{}-{}.toml",
        std::process::id(),
        "config"
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&config);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);

    let add = devrelay()
        .args([
            "project",
            "add",
            root.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(add.status.success());
    let add_stdout = String::from_utf8(add.stdout).unwrap();
    assert!(add_stdout.contains("\"added\""));

    let list = devrelay()
        .args([
            "projects",
            "list",
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(list.status.success());
    let list_stdout = String::from_utf8(list.stdout).unwrap();
    assert!(list_stdout.contains("\"project_id\""));

    let project_name = root.file_name().unwrap().to_str().unwrap();
    let show = devrelay()
        .args([
            "project",
            "show",
            project_name,
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(show.status.success());

    let remove = devrelay()
        .args([
            "project",
            "remove",
            project_name,
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(remove.status.success());

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_file(config);
}

#[test]
fn project_add_uses_manifest_and_records_fingerprints() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-project-manifest-test-{}-{}",
        std::process::id(),
        "repo"
    ));
    let config = std::env::temp_dir().join(format!(
        "devrelay-project-manifest-test-{}-{}.toml",
        std::process::id(),
        "config"
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&config);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.com/devrelay/demo.git",
        ],
    );
    write_manifest(&root, "manifest-project", "Manifest Project");

    let add = devrelay()
        .args([
            "project",
            "add",
            root.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(add.status.success());
    let add_json: serde_json::Value = serde_json::from_slice(&add.stdout).unwrap();
    let added = &add_json["added"];
    assert_eq!(added["project_id"].as_str(), Some("manifest-project"));
    assert_eq!(added["display_name"].as_str(), Some("Manifest Project"));
    let canonical_manifest = root.canonicalize().unwrap().join("devrelay.toml");
    assert_eq!(
        added["manifest_path"].as_str(),
        Some(canonical_manifest.to_str().unwrap())
    );
    assert!(
        added["remote_url_fingerprint"]
            .as_str()
            .is_some_and(|value| value.starts_with("remote_"))
    );
    assert!(
        added["root_commit_fingerprint"]
            .as_str()
            .is_some_and(|value| value.starts_with("root_"))
    );
    let workspaces = added["workspaces"].as_object().unwrap();
    assert_eq!(workspaces.len(), 1);
    let workspace = workspaces.values().next().unwrap();
    assert!(
        workspace["workspace_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("w_"))
    );
    assert_eq!(workspace["project_id"].as_str(), Some("manifest-project"));
    assert_eq!(workspace["device_id"].as_str(), Some("local-device"));
    assert_eq!(
        workspace["local_path"].as_str(),
        Some(root.canonicalize().unwrap().to_str().unwrap())
    );
    assert!(workspace["platform_profile"].as_str().is_some());
    assert_eq!(workspace["state"].as_str(), Some("active"));
    assert!(workspace["last_seen_head"].as_str().is_some());
    assert!(workspace["last_checkpoint_id"].is_null());

    let duplicate = devrelay()
        .args([
            "--json-errors",
            "project",
            "add",
            root.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    let stderr = String::from_utf8(duplicate.stderr).unwrap();
    assert!(stderr.contains("DR-CONFIG"));
    assert!(stderr.contains("already registered"));

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_file(config);
}

#[test]
fn project_add_appends_workspaces_and_workspace_remove_handles_stale_paths() {
    let root_a = std::env::temp_dir().join(format!(
        "devrelay-workspace-test-{}-{}",
        std::process::id(),
        "repo-a"
    ));
    let root_b = std::env::temp_dir().join(format!(
        "devrelay-workspace-test-{}-{}",
        std::process::id(),
        "repo-b"
    ));
    let config = std::env::temp_dir().join(format!(
        "devrelay-workspace-test-{}-{}.toml",
        std::process::id(),
        "config"
    ));
    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
    let _ = std::fs::remove_file(&config);
    std::fs::create_dir(&root_a).unwrap();
    std::fs::create_dir(&root_b).unwrap();
    init_git_repo(&root_a);
    init_git_repo(&root_b);
    write_manifest(&root_a, "shared-project", "Shared Project");
    write_manifest(&root_b, "shared-project", "Shared Project");

    let add_a = devrelay()
        .args([
            "project",
            "add",
            root_a.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(add_a.status.success());
    let add_a_json: serde_json::Value = serde_json::from_slice(&add_a.stdout).unwrap();
    let workspace_id_a = add_a_json["added"]["workspaces"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .to_string();

    let add_b = devrelay()
        .args([
            "project",
            "add",
            root_b.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(add_b.status.success());
    let add_b_json: serde_json::Value = serde_json::from_slice(&add_b.stdout).unwrap();
    assert_eq!(
        add_b_json["added"]["workspaces"].as_object().unwrap().len(),
        2
    );

    std::fs::remove_dir_all(&root_a).unwrap();
    let list = devrelay()
        .args([
            "projects",
            "list",
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(list.status.success());
    let list_json: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(
        list_json[0]["workspaces"][&workspace_id_a]["state"].as_str(),
        Some("stale")
    );

    let remove = devrelay()
        .args([
            "workspace",
            "remove",
            &workspace_id_a,
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(remove.status.success());

    let show = devrelay()
        .args([
            "project",
            "show",
            "shared-project",
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(show.status.success());
    let show_json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    let workspaces = show_json["workspaces"].as_object().unwrap();
    assert_eq!(workspaces.len(), 1);
    assert!(!workspaces.contains_key(&workspace_id_a));
    assert_eq!(
        show_json["local_path"].as_str(),
        Some(root_b.canonicalize().unwrap().to_str().unwrap())
    );

    let _ = std::fs::remove_dir_all(root_b);
    let _ = std::fs::remove_file(config);
}

#[test]
fn checkpoint_persists_snapshot_store_and_exports_json() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-snapshot-store-test-{}-{}",
        std::process::id(),
        "repo"
    ));
    let home = std::env::temp_dir().join(format!(
        "devrelay-snapshot-store-test-{}-{}",
        std::process::id(),
        "home"
    ));
    let out = std::env::temp_dir().join(format!(
        "devrelay-snapshot-store-test-{}-{}.json",
        std::process::id(),
        "checkpoint"
    ));
    let export = std::env::temp_dir().join(format!(
        "devrelay-snapshot-store-test-{}-{}.json",
        std::process::id(),
        "export"
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&export);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);
    write_manifest(&root, "cli-store-project", "CLI Store Project");
    std::fs::write(root.join("README.md"), "changed\n").unwrap();

    let checkpoint = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "checkpoint",
            "--repo",
            root.to_str().unwrap(),
            "--manifest",
            root.join("devrelay.toml").to_str().unwrap(),
            "--label",
            "first",
            "--pin",
            "--out",
            out.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(checkpoint.status.success());
    let checkpoint_json: serde_json::Value = serde_json::from_slice(&checkpoint.stdout).unwrap();
    let stored = &checkpoint_json["checkpoint"];
    let snapshot_id = stored["snapshot_id"].as_str().unwrap();
    assert_eq!(stored["sequence_number"].as_i64(), Some(1));
    assert_eq!(stored["pinned"].as_bool(), Some(true));
    assert_eq!(stored["label"].as_str(), Some("first"));
    assert!(out.exists());
    assert!(
        home.join("projects")
            .join("cli-store-project")
            .join("snapshots.git")
            .join("HEAD")
            .exists()
    );
    assert!(
        home.join("projects")
            .join("cli-store-project")
            .join("metadata.sqlite")
            .exists()
    );

    let source_index_ref = stored["metadata"]["index_ref"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("refs/devrelay/snapshots/{snapshot_id}/index"));
    let source_ref = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "--verify", &source_index_ref])
        .output()
        .unwrap();
    assert!(!source_ref.status.success());

    let list = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "snapshot",
            "list",
            "--project",
            "cli-store-project",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(list.status.success());
    let list_json: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(list_json.as_array().unwrap().len(), 1);
    assert_eq!(list_json[0]["snapshot_id"].as_str(), Some(snapshot_id));

    let show = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "snapshot",
            "show",
            snapshot_id,
            "--project",
            "cli-store-project",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(show.status.success());
    let show_json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(show_json["snapshot_id"].as_str(), Some(snapshot_id));
    assert_eq!(show_json["label"].as_str(), Some("first"));

    let exported = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "snapshot",
            "export",
            snapshot_id,
            "--project",
            "cli-store-project",
            "--out",
            export.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(exported.status.success());
    assert!(export.exists());

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_file(out);
    let _ = std::fs::remove_file(export);
}

#[test]
fn recover_list_show_and_open_restores_snapshot_without_touching_source() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-recover-test-{}-{}",
        std::process::id(),
        "source"
    ));
    let target = std::env::temp_dir().join(format!(
        "devrelay-recover-test-{}-{}",
        std::process::id(),
        "target"
    ));
    let home = std::env::temp_dir().join(format!(
        "devrelay-recover-test-{}-{}",
        std::process::id(),
        "home"
    ));
    let config = std::env::temp_dir().join(format!(
        "devrelay-recover-test-{}-{}.toml",
        std::process::id(),
        "config"
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&target);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_file(&config);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);
    write_manifest(&root, "recover-project", "Recover Project");

    let add = devrelay()
        .args([
            "project",
            "add",
            root.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(add.status.success());

    std::fs::write(root.join("README.md"), "source changed\n").unwrap();
    std::fs::write(root.join("notes.md"), "recover me\n").unwrap();
    let checkpoint = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "checkpoint",
            "--repo",
            root.to_str().unwrap(),
            "--manifest",
            root.join("devrelay.toml").to_str().unwrap(),
            "--label",
            "recoverable",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(checkpoint.status.success());
    let checkpoint_json: serde_json::Value = serde_json::from_slice(&checkpoint.stdout).unwrap();
    let snapshot_id = checkpoint_json["checkpoint"]["snapshot_id"]
        .as_str()
        .unwrap();
    let source_readme = std::fs::read_to_string(root.join("README.md")).unwrap();

    let list_all = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "recover",
            "list",
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(list_all.status.success());
    let list_all_json: serde_json::Value = serde_json::from_slice(&list_all.stdout).unwrap();
    assert_eq!(list_all_json.as_array().unwrap().len(), 1);

    let list_project = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "recover",
            "list",
            "--project",
            "recover-project",
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(list_project.status.success());

    let show = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "recover",
            "show",
            snapshot_id,
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(show.status.success());
    let show_json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(show_json["snapshot_id"].as_str(), Some(snapshot_id));

    let open = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "recover",
            "open",
            snapshot_id,
            "--path",
            target.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--register",
            "--name",
            "review-copy",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(open.status.success());
    let open_json: serde_json::Value = serde_json::from_slice(&open.stdout).unwrap();
    assert_eq!(open_json["name"].as_str(), Some("review-copy"));
    assert!(
        open_json["registered"]["workspace_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("w_"))
    );
    assert_eq!(
        std::fs::read_to_string(target.join("README.md")).unwrap(),
        "source changed\n"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("notes.md")).unwrap(),
        "recover me\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("README.md")).unwrap(),
        source_readme
    );

    let project = devrelay()
        .args([
            "project",
            "show",
            "recover-project",
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(project.status.success());
    let project_json: serde_json::Value = serde_json::from_slice(&project.stdout).unwrap();
    assert_eq!(project_json["workspaces"].as_object().unwrap().len(), 2);

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(target);
    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_file(config);
}

#[test]
fn recover_show_reports_missing_snapshot() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-recover-missing-test-{}-{}",
        std::process::id(),
        "source"
    ));
    let home = std::env::temp_dir().join(format!(
        "devrelay-recover-missing-test-{}-{}",
        std::process::id(),
        "home"
    ));
    let config = std::env::temp_dir().join(format!(
        "devrelay-recover-missing-test-{}-{}.toml",
        std::process::id(),
        "config"
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_file(&config);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);
    write_manifest(&root, "recover-missing-project", "Recover Missing Project");
    let add = devrelay()
        .args([
            "project",
            "add",
            root.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(add.status.success());

    let output = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--json-errors",
            "recover",
            "show",
            "s1_000000000000000000000000",
            "--config",
            config.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("DR-RECOVER-SNAPSHOT-NOT-FOUND"));
    assert!(stderr.contains("unknown snapshot"));

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_file(config);
}

#[test]
fn recover_open_refuses_dirty_target() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-recover-dirty-test-{}-{}",
        std::process::id(),
        "source"
    ));
    let target = std::env::temp_dir().join(format!(
        "devrelay-recover-dirty-test-{}-{}",
        std::process::id(),
        "target"
    ));
    let home = std::env::temp_dir().join(format!(
        "devrelay-recover-dirty-test-{}-{}",
        std::process::id(),
        "home"
    ));
    let config = std::env::temp_dir().join(format!(
        "devrelay-recover-dirty-test-{}-{}.toml",
        std::process::id(),
        "config"
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&target);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_file(&config);
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&target).unwrap();
    init_git_repo(&root);
    init_git_repo(&target);
    write_manifest(&root, "recover-dirty-project", "Recover Dirty Project");
    let add = devrelay()
        .args([
            "project",
            "add",
            root.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(add.status.success());
    std::fs::write(root.join("README.md"), "source changed\n").unwrap();
    let checkpoint = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "checkpoint",
            "--repo",
            root.to_str().unwrap(),
            "--manifest",
            root.join("devrelay.toml").to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(checkpoint.status.success());
    let checkpoint_json: serde_json::Value = serde_json::from_slice(&checkpoint.stdout).unwrap();
    let snapshot_id = checkpoint_json["checkpoint"]["snapshot_id"]
        .as_str()
        .unwrap();
    std::fs::write(target.join("dirty.txt"), "local\n").unwrap();

    let output = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--json-errors",
            "recover",
            "open",
            snapshot_id,
            "--path",
            target.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("DR-APPLY-DIRTY-TARGET"));

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(target);
    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_file(config);
}

#[test]
fn apply_dirty_policy_snapshots_backup_and_can_use_new_workspace() {
    let source = std::env::temp_dir().join(format!(
        "devrelay-dirty-policy-test-{}-{}",
        std::process::id(),
        "source"
    ));
    let target = std::env::temp_dir().join(format!(
        "devrelay-dirty-policy-test-{}-{}",
        std::process::id(),
        "target"
    ));
    let target_new = std::env::temp_dir().join(format!(
        "devrelay-dirty-policy-test-{}-{}",
        std::process::id(),
        "target-new"
    ));
    let backup_recover = std::env::temp_dir().join(format!(
        "devrelay-dirty-policy-test-{}-{}",
        std::process::id(),
        "backup-recover"
    ));
    let home = std::env::temp_dir().join(format!(
        "devrelay-dirty-policy-test-{}-{}",
        std::process::id(),
        "home"
    ));
    let config = std::env::temp_dir().join(format!(
        "devrelay-dirty-policy-test-{}-{}.toml",
        std::process::id(),
        "config"
    ));
    let snapshot_file = std::env::temp_dir().join(format!(
        "devrelay-dirty-policy-test-{}-{}.json",
        std::process::id(),
        "snapshot"
    ));
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&target);
    let _ = std::fs::remove_dir_all(&target_new);
    let _ = std::fs::remove_dir_all(&backup_recover);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_file(&config);
    let _ = std::fs::remove_file(&snapshot_file);
    std::fs::create_dir(&source).unwrap();
    init_git_repo(&source);
    write_manifest(&source, "dirty-policy-project", "Dirty Policy Project");

    let add = devrelay()
        .args([
            "project",
            "add",
            source.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(add.status.success());
    assert!(
        Command::new("git")
            .arg("clone")
            .arg(&source)
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("clone")
            .arg(&source)
            .arg(&target_new)
            .status()
            .unwrap()
            .success()
    );

    std::fs::write(source.join("README.md"), "source snapshot\n").unwrap();
    let checkpoint = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "checkpoint",
            "--repo",
            source.to_str().unwrap(),
            "--manifest",
            source.join("devrelay.toml").to_str().unwrap(),
            "--out",
            snapshot_file.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(checkpoint.status.success());
    let checkpoint_json: serde_json::Value = serde_json::from_slice(&checkpoint.stdout).unwrap();
    let snapshot_repo = checkpoint_json["snapshot_repo"].as_str().unwrap();

    std::fs::write(target.join("target-only.txt"), "preserve me\n").unwrap();
    let apply = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "apply",
            "--repo",
            target.to_str().unwrap(),
            "--source",
            snapshot_repo,
            "--snapshot",
            snapshot_file.to_str().unwrap(),
            "--dirty-policy",
            "snapshot-and-fork",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(apply.status.success());
    let apply_json: serde_json::Value = serde_json::from_slice(&apply.stdout).unwrap();
    assert_eq!(
        apply_json["dirty_policy"].as_str(),
        Some("snapshot-and-fork")
    );
    assert_eq!(apply_json["backup"]["pinned"].as_bool(), Some(true));
    assert!(
        apply_json["backup"]["session_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("fork_"))
    );
    let backup_snapshot_id = apply_json["backup"]["snapshot_id"].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("README.md")).unwrap(),
        "source snapshot\n"
    );

    let recover_backup = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "recover",
            "open",
            backup_snapshot_id,
            "--project",
            "dirty-policy-project",
            "--path",
            backup_recover.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(recover_backup.status.success());
    assert_eq!(
        std::fs::read_to_string(backup_recover.join("target-only.txt")).unwrap(),
        "preserve me\n"
    );

    std::fs::write(target_new.join("new-workspace-only.txt"), "stay here\n").unwrap();
    let apply_new = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "apply",
            "--repo",
            target_new.to_str().unwrap(),
            "--source",
            snapshot_repo,
            "--snapshot",
            snapshot_file.to_str().unwrap(),
            "--dirty-policy",
            "new-workspace",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(apply_new.status.success());
    let apply_new_json: serde_json::Value = serde_json::from_slice(&apply_new.stdout).unwrap();
    let applied_repo = std::path::PathBuf::from(apply_new_json["applied_repo"].as_str().unwrap());
    assert!(target_new.join("new-workspace-only.txt").exists());
    assert_eq!(
        std::fs::read_to_string(applied_repo.join("README.md")).unwrap(),
        "source snapshot\n"
    );

    let _ = std::fs::remove_dir_all(source);
    let _ = std::fs::remove_dir_all(target);
    let _ = std::fs::remove_dir_all(target_new);
    let _ = std::fs::remove_dir_all(backup_recover);
    let _ = std::fs::remove_dir_all(applied_repo);
    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_file(config);
    let _ = std::fs::remove_file(snapshot_file);
}

#[test]
fn continue_dry_run_and_clean_target_handoff_updates_workspace_states() {
    let source = std::env::temp_dir().join(format!(
        "devrelay-continue-clean-test-{}-{}",
        std::process::id(),
        "source"
    ));
    let target = std::env::temp_dir().join(format!(
        "devrelay-continue-clean-test-{}-{}",
        std::process::id(),
        "target"
    ));
    let home = std::env::temp_dir().join(format!(
        "devrelay-continue-clean-test-{}-{}",
        std::process::id(),
        "home"
    ));
    let config = std::env::temp_dir().join(format!(
        "devrelay-continue-clean-test-{}-{}.toml",
        std::process::id(),
        "config"
    ));
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&target);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_file(&config);
    std::fs::create_dir(&source).unwrap();
    init_git_repo(&source);
    write_manifest(&source, "continue-clean-project", "Continue Clean Project");
    assert!(
        Command::new("git")
            .arg("clone")
            .arg(&source)
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    for path in [&source, &target] {
        let mut command = devrelay();
        command.args([
            "project",
            "add",
            path.to_str().unwrap(),
            "--manifest",
            source.join("devrelay.toml").to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ]);
        assert!(command.output().unwrap().status.success());
    }

    std::fs::write(source.join("README.md"), "handoff clean\n").unwrap();
    let dry_run = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "continue",
            "--source",
            source.to_str().unwrap(),
            "--target",
            target.to_str().unwrap(),
            "--manifest",
            source.join("devrelay.toml").to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(dry_run.status.success());
    let dry_json: serde_json::Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(dry_json["dry_run"].as_bool(), Some(true));
    assert_eq!(
        std::fs::read_to_string(target.join("README.md")).unwrap(),
        "demo\n"
    );

    let continued = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "continue",
            "--source",
            source.to_str().unwrap(),
            "--target",
            target.to_str().unwrap(),
            "--manifest",
            source.join("devrelay.toml").to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(continued.status.success());
    assert_eq!(
        std::fs::read_to_string(target.join("README.md")).unwrap(),
        "handoff clean\n"
    );

    let project = devrelay()
        .args([
            "project",
            "show",
            "continue-clean-project",
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(project.status.success());
    let project_json: serde_json::Value = serde_json::from_slice(&project.stdout).unwrap();
    assert_eq!(
        workspace_state_for(&project_json, &source).as_deref(),
        Some("inactive")
    );
    assert_eq!(
        workspace_state_for(&project_json, &target).as_deref(),
        Some("active")
    );

    let _ = std::fs::remove_dir_all(source);
    let _ = std::fs::remove_dir_all(target);
    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_file(config);
}

#[test]
fn continue_dirty_target_uses_backup_policy() {
    let source = std::env::temp_dir().join(format!(
        "devrelay-continue-dirty-test-{}-{}",
        std::process::id(),
        "source"
    ));
    let target = std::env::temp_dir().join(format!(
        "devrelay-continue-dirty-test-{}-{}",
        std::process::id(),
        "target"
    ));
    let backup_recover = std::env::temp_dir().join(format!(
        "devrelay-continue-dirty-test-{}-{}",
        std::process::id(),
        "backup-recover"
    ));
    let home = std::env::temp_dir().join(format!(
        "devrelay-continue-dirty-test-{}-{}",
        std::process::id(),
        "home"
    ));
    let config = std::env::temp_dir().join(format!(
        "devrelay-continue-dirty-test-{}-{}.toml",
        std::process::id(),
        "config"
    ));
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&target);
    let _ = std::fs::remove_dir_all(&backup_recover);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_file(&config);
    std::fs::create_dir(&source).unwrap();
    init_git_repo(&source);
    write_manifest(&source, "continue-dirty-project", "Continue Dirty Project");
    assert!(
        Command::new("git")
            .arg("clone")
            .arg(&source)
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    for path in [&source, &target] {
        let mut command = devrelay();
        command.args([
            "project",
            "add",
            path.to_str().unwrap(),
            "--manifest",
            source.join("devrelay.toml").to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ]);
        assert!(command.output().unwrap().status.success());
    }

    std::fs::write(source.join("README.md"), "handoff dirty\n").unwrap();
    std::fs::write(target.join("target-only.txt"), "dirty target\n").unwrap();
    let continued = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "continue",
            "--source",
            source.to_str().unwrap(),
            "--target",
            target.to_str().unwrap(),
            "--manifest",
            source.join("devrelay.toml").to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--dirty-policy",
            "snapshot-and-fork",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(continued.status.success());
    let continued_json: serde_json::Value = serde_json::from_slice(&continued.stdout).unwrap();
    assert_eq!(continued_json["backup"]["pinned"].as_bool(), Some(true));
    let backup_snapshot_id = continued_json["backup"]["snapshot_id"].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("README.md")).unwrap(),
        "handoff dirty\n"
    );

    let backup = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "recover",
            "open",
            backup_snapshot_id,
            "--project",
            "continue-dirty-project",
            "--path",
            backup_recover.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(backup.status.success());
    assert_eq!(
        std::fs::read_to_string(backup_recover.join("target-only.txt")).unwrap(),
        "dirty target\n"
    );

    let _ = std::fs::remove_dir_all(source);
    let _ = std::fs::remove_dir_all(target);
    let _ = std::fs::remove_dir_all(backup_recover);
    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_file(config);
}

#[test]
fn project_add_rejects_non_git_path() {
    let root = std::env::temp_dir().join(format!("devrelay-not-git-test-{}", std::process::id()));
    let config =
        std::env::temp_dir().join(format!("devrelay-not-git-test-{}.toml", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&config);
    std::fs::create_dir(&root).unwrap();

    let output = devrelay()
        .args([
            "--json-errors",
            "project",
            "add",
            root.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("DR-GIT-NOT-REPOSITORY"));

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_file(config);
}
