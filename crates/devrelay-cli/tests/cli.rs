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

#[test]
fn prints_version() {
    let output = devrelay().arg("--version").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("devrelay"));
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
    assert!(stderr.contains("DR-CONFIG"));
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
