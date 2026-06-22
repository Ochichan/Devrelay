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
    std::fs::write(
        root.join("devrelay.toml"),
        r#"schema = 1
project_id = "manifest-project"
name = "Manifest Project"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
    )
    .unwrap();

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
