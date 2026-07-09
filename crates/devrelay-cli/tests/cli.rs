use std::path::Path;
use std::process::Command;

fn devrelay() -> Command {
    Command::new(env!("CARGO_BIN_EXE_devrelay"))
}

#[cfg(unix)]
fn devrelay_agent() -> Command {
    let cli_path = std::path::PathBuf::from(env!("CARGO_BIN_EXE_devrelay"));
    let agent_path = std::env::var_os("CARGO_BIN_EXE_devrelay-agent")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| cli_path.with_file_name("devrelay-agent"));
    assert!(
        agent_path.exists(),
        "devrelay-agent binary not found at {}; run cargo test --workspace",
        agent_path.display()
    );
    Command::new(agent_path)
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

fn read_text_lf(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

fn comparable_path(path: impl AsRef<Path>) -> String {
    let raw = path.as_ref().to_string_lossy().replace('\\', "/");
    raw.strip_prefix("//?/").unwrap_or(&raw).to_string()
}

fn assert_path_value(actual: Option<&str>, expected: &Path) {
    assert_eq!(
        actual.map(|value| comparable_path(Path::new(value))),
        Some(comparable_path(expected))
    );
}

fn init_git_repo(root: &Path) {
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "core.eol", "lf"]);
    git(root, &["config", "user.name", "DevRelay Test"]);
    git(
        root,
        &["config", "user.email", "devrelay-test@example.local"],
    );
    std::fs::write(root.join("README.md"), "demo\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "base"]);
}

#[cfg(unix)]
#[test]
fn agent_install_dry_run_renders_service_template() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-agent-install-dry-run-test-{}",
        std::process::id()
    ));
    let service_dir = root.join("services");
    let agent_bin = root.join("bin").join("devrelay-agent");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(agent_bin.parent().unwrap()).unwrap();

    let output = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "agent",
            "install",
            "--dry-run",
            "--service-dir",
            service_dir.to_str().unwrap(),
            "--agent-bin",
            agent_bin.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["dry_run"], true);
    assert_eq!(value["installed"], false);
    assert!(
        value["service_path"]
            .as_str()
            .unwrap()
            .starts_with(service_dir.to_str().unwrap())
    );
    assert!(
        value["content"]
            .as_str()
            .unwrap()
            .contains(agent_bin.to_str().unwrap())
    );
    assert!(!Path::new(value["service_path"].as_str().unwrap()).exists());

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn agent_install_status_and_uninstall_round_trip() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-agent-install-test-{}",
        std::process::id()
    ));
    let service_dir = root.join("services");
    let agent_bin = root.join("bin").join("devrelay-agent");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(agent_bin.parent().unwrap()).unwrap();

    let install = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "agent",
            "install",
            "--service-dir",
            service_dir.to_str().unwrap(),
            "--agent-bin",
            agent_bin.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        install.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&install.stderr)
    );
    let installed: serde_json::Value = serde_json::from_slice(&install.stdout).unwrap();
    let service_path = std::path::PathBuf::from(installed["service_path"].as_str().unwrap());
    assert_eq!(installed["installed"], true);
    assert!(service_path.exists());

    let status = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "agent",
            "status",
            "--service-dir",
            service_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["installed"], true);
    assert_eq!(status["service_path"], service_path.to_str().unwrap());

    let uninstall = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "agent",
            "uninstall",
            "--service-dir",
            service_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(uninstall.status.success());
    let uninstalled: serde_json::Value = serde_json::from_slice(&uninstall.stdout).unwrap();
    assert_eq!(uninstalled["removed"], true);
    assert!(!service_path.exists());

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn safety_diagnostics_redacted_by_default_for_cli_export() {
    // Invariant: safety/diagnostics_redacted_by_default.
    let mut running = RunningCliAgent::start("devrelay-cli-diagnostics-test");
    let out = running.root.join("diagnostics").join("bundle.json");

    let output = devrelay()
        .env("DEVRELAY_HOME", &running.root)
        .args([
            "--agent-socket",
            running.socket.to_str().unwrap(),
            "diagnostics",
            "export",
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
    let exported: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(exported["path"], out.to_str().unwrap());
    assert_eq!(exported["include_sensitive_paths"], false);
    assert_eq!(exported["source_code_included"], false);
    assert_eq!(exported["snapshot_objects_included"], false);

    let raw_bundle = std::fs::read_to_string(&out).unwrap();
    assert!(
        !raw_bundle.contains(running.root.to_str().unwrap()),
        "bundle should redact DEVRELAY_HOME paths by default"
    );
    let bundle: serde_json::Value = serde_json::from_str(&raw_bundle).unwrap();
    assert!(bundle["version"].as_str().is_some());
    assert!(bundle["protocol_version"].as_u64().is_some());
    assert_eq!(bundle["include_sensitive_paths"], false);
    assert!(bundle["config"].as_object().is_some());
    assert_eq!(bundle["capabilities"]["structured_logs"], true);
    assert!(
        bundle["capabilities"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method == "diagnostics.export")
    );
    assert!(bundle["timing"]["duration_millis"].as_u64().is_some());
    assert!(
        !bundle["recent_structured_logs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        bundle["state_machine_records"]["leases"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        bundle["git_command_exit_codes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(bundle["source_code_included"], false);
    assert_eq!(bundle["snapshot_objects_included"], false);

    let sensitive_out = running
        .root
        .join("diagnostics")
        .join("sensitive-bundle.json");
    let sensitive_output = devrelay()
        .env("DEVRELAY_HOME", &running.root)
        .args([
            "--agent-socket",
            running.socket.to_str().unwrap(),
            "diagnostics",
            "export",
            "--out",
            sensitive_out.to_str().unwrap(),
            "--include-sensitive-paths",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        sensitive_output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&sensitive_output.stderr)
    );
    let sensitive_exported: serde_json::Value =
        serde_json::from_slice(&sensitive_output.stdout).unwrap();
    assert_eq!(sensitive_exported["include_sensitive_paths"], true);
    assert_eq!(sensitive_exported["source_code_included"], false);
    assert_eq!(sensitive_exported["snapshot_objects_included"], false);
    let sensitive_bundle: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sensitive_out).unwrap()).unwrap();
    assert_eq!(sensitive_bundle["include_sensitive_paths"], true);
    assert_eq!(sensitive_bundle["source_code_included"], false);
    assert_eq!(sensitive_bundle["snapshot_objects_included"], false);

    running.stop();
}

#[test]
fn doctor_git_performance_reports_and_preserves_user_config() {
    let repo = std::env::temp_dir().join(format!(
        "devrelay-doctor-git-performance-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    git(&repo, &["config", "core.untrackedCache", "false"]);

    let output = devrelay()
        .args([
            "doctor",
            "git-performance",
            "--repo",
            repo.to_str().unwrap(),
            "--fix-safe",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        value["git_version"]
            .as_str()
            .unwrap()
            .starts_with("git version ")
    );
    assert_eq!(value["untracked_cache_config"], "false");
    assert!(
        value["skipped_fixes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|fix| fix["key"] == "core.untrackedCache")
    );

    let config = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["config", "--local", "--get", "core.untrackedCache"])
        .output()
        .unwrap();
    assert!(config.status.success());
    assert_eq!(String::from_utf8_lossy(&config.stdout).trim(), "false");

    let _ = std::fs::remove_dir_all(repo);
}

#[cfg(not(windows))]
#[test]
fn doctor_paths_reports_tracked_and_untracked_portability_issues() {
    let repo =
        std::env::temp_dir().join(format!("devrelay-doctor-paths-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
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
    std::fs::write(repo.join("CON.txt"), "reserved\n").unwrap();
    git(&repo, &["add", "devrelay.toml", "CON.txt"]);
    git(&repo, &["commit", "-m", "manifest and reserved path"]);
    std::fs::write(repo.join("scratch?.txt"), "accepted untracked\n").unwrap();

    let output = devrelay()
        .args([
            "doctor",
            "paths",
            "--repo",
            repo.to_str().unwrap(),
            "--target-platform",
            "windows-native-x86_64",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["target_platform_key"], "windows-native-x86_64");
    assert_eq!(value["accepted_untracked_count"], 1);
    let issues = value["issues"].as_array().unwrap();
    assert!(issues.iter().any(|issue| {
        issue["code"] == "windows-reserved-name"
            && issue["path"] == "CON.txt"
            && issue["source"] == "tracked"
    }));
    assert!(issues.iter().any(|issue| {
        issue["code"] == "windows-invalid-character"
            && issue["path"] == "scratch?.txt"
            && issue["source"] == "accepted-untracked"
    }));

    let _ = std::fs::remove_dir_all(repo);
}

#[cfg(unix)]
#[test]
fn doctor_paths_reports_symlink_capability_mismatch() {
    let repo = std::env::temp_dir().join(format!(
        "devrelay-doctor-symlink-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
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
    std::os::unix::fs::symlink("README.md", repo.join("readme-link")).unwrap();
    git(&repo, &["add", "devrelay.toml", "readme-link"]);
    git(&repo, &["commit", "-m", "manifest and symlink"]);

    let output = devrelay()
        .args([
            "doctor",
            "paths",
            "--repo",
            repo.to_str().unwrap(),
            "--target-platform",
            "windows-native-x86_64",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issues = value["issues"].as_array().unwrap();
    assert!(issues.iter().any(|issue| {
        issue["code"] == "symlink-unsupported-on-target" && issue["path"] == "readme-link"
    }));

    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn doctor_line_endings_reports_missing_policy_and_target_risk() {
    let repo = std::env::temp_dir().join(format!(
        "devrelay-doctor-line-endings-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);

    let output = devrelay()
        .args([
            "doctor",
            "line-endings",
            "--repo",
            repo.to_str().unwrap(),
            "--target-platform",
            "windows-native-x86_64",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["target_platform_key"], "windows-native-x86_64");
    assert_eq!(value["gitattributes_present"], false);
    assert!(
        value["tracked_file_count"].as_u64().unwrap() >= 1,
        "tracked_file_count should include README.md"
    );
    let warnings = value["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| { warning["code"] == "missing-gitattributes-policy" })
    );
    assert!(
        warnings
            .iter()
            .any(|warning| { warning["code"] == "risky-target-line-ending-config" })
    );

    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn doctor_wsl_filesystem_reports_device_mapping_guidance() {
    let repo = std::env::temp_dir().join(format!(
        "devrelay-doctor-wsl-filesystem-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);

    let output = devrelay()
        .args([
            "doctor",
            "wsl-filesystem",
            "--repo",
            repo.to_str().unwrap(),
            "--platform-key",
            "wsl2-linux-gnu-x86_64",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["platform_key"], "wsl2-linux-gnu-x86_64");
    assert!(value["guidance"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .unwrap()
            .contains("different DevRelay devices")
    }));

    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn doctor_environment_reports_missing_required_secret() {
    let repo = std::env::temp_dir().join(format!(
        "devrelay-doctor-environment-test-{}",
        std::process::id()
    ));
    let home = repo.join("home");
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    std::fs::write(
        repo.join("devrelay.toml"),
        r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"

[secrets.api_token]
target = ".devrelay/secrets/api_token"
required = true
"#,
    )
    .unwrap();

    let output = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "doctor",
            "environment",
            "--repo",
            repo.to_str().unwrap(),
            "--platform-key",
            "darwin-arm64",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["platform_key"], "darwin-arm64");
    assert_eq!(value["required_secret_count"], 1);
    assert_eq!(value["mapped_required_secret_count"], 0);
    assert!(value["issues"].as_array().unwrap().iter().any(|issue| {
        issue["code"] == "missing-required-secret" && issue["secret_name"] == "api_token"
    }));

    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn doctor_project_safety_reports_pending_changes() {
    let repo = std::env::temp_dir().join(format!(
        "devrelay-doctor-project-safety-test-{}",
        std::process::id()
    ));
    let home = repo.join("home");
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    write_manifest(&repo, "safety-project", "Safety Project");
    std::fs::write(repo.join("pending.txt"), "pending\n").unwrap();

    let output = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "doctor",
            "project-safety",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["project_id"], "safety-project");
    assert_eq!(value["status"]["clean"], false);
    assert_eq!(value["status"]["counts"]["untracked"], 2);
    assert!(value["issues"].as_array().unwrap().iter().any(|issue| {
        issue["code"] == "pending-changes"
            && issue["safe_actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| action.as_str().unwrap().contains("devrelay checkpoint"))
    }));

    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn doctor_secrets_reports_missing_required_secret() {
    let repo = std::env::temp_dir().join(format!(
        "devrelay-doctor-secrets-test-{}",
        std::process::id()
    ));
    let home = repo.join("home");
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    std::fs::write(
        repo.join("devrelay.toml"),
        r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"

[secrets.api_token]
target = "API_TOKEN"
mode = "environment"
required = true
"#,
    )
    .unwrap();

    let output = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "doctor",
            "secrets",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["required_secret_count"], 1);
    assert_eq!(value["mapped_required_secret_count"], 0);
    assert_eq!(value["missing_required_secret_count"], 1);
    assert!(value["issues"].as_array().unwrap().iter().any(|issue| {
        issue["code"] == "missing-required-secret" && issue["secret_name"] == "api_token"
    }));

    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn environment_status_reports_persisted_hydration_state() {
    use devrelay_core::{DevRelayHome, HydrationState, HydrationStateRecord, save_hydration_state};

    let root = std::env::temp_dir().join(format!(
        "devrelay-environment-status-test-{}",
        std::process::id()
    ));
    let repo = root.join("repo");
    let home = root.join("home");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    write_manifest(&repo, "env-status-project", "Environment Status Project");
    git(&repo, &["add", "devrelay.toml"]);
    git(&repo, &["commit", "-m", "manifest"]);

    let add = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "project",
            "add",
            repo.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );
    let project: serde_json::Value = serde_json::from_slice(&add.stdout).unwrap();
    let workspace_id = project["added"]["workspaces"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .to_string();

    let devrelay_home = DevRelayHome::new(&home);
    let mut record =
        HydrationStateRecord::new("env-status-project", Some(workspace_id.clone()), 456);
    record.state = HydrationState::AppReady;
    record.attempt = 3;
    save_hydration_state(
        &devrelay_home.hydration_state_path("env-status-project", Some(&workspace_id)),
        &record,
    )
    .unwrap();

    let status = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "environment",
            "status",
            "--project",
            "env-status-project",
            "--workspace",
            &workspace_id,
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&status.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["environments"][0]["project_id"], "env-status-project");
    assert_eq!(value["environments"][0]["workspace_id"], workspace_id);
    assert_eq!(value["environments"][0]["state"], "app-ready");
    assert_eq!(value["environments"][0]["attempt"], 3);
    assert_eq!(value["environments"][0]["persisted"], true);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn audit_list_and_export_redact_by_default() {
    use devrelay_core::{AuditEventInput, AuditEventType, AuditOutcome, DevRelayHome, MetadataDb};

    let home = std::env::temp_dir().join(format!("devrelay-audit-test-{}", std::process::id()));
    let repo = home.join("repo");
    let out = home.join("audit-export.json");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    write_manifest(&repo, "audit-project", "Audit Project");
    git(&repo, &["add", "devrelay.toml"]);
    git(&repo, &["commit", "-m", "manifest"]);

    let add = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "project",
            "add",
            repo.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );

    let devrelay_home = DevRelayHome::new(&home);
    let db = MetadataDb::open(devrelay_home.metadata_db_path("audit-project")).unwrap();
    let mut event = AuditEventInput::new(
        AuditEventType::SecurityBlocked,
        AuditOutcome::Blocked,
        "blocked secret-like path",
    )
    .with_detail(serde_json::json!({
        "target_path": repo.join(".env").to_str().unwrap(),
        "api_token": "secret-token",
    }));
    event.project_id = Some("audit-project".to_string());
    event.snapshot_id = Some("s1_audit".to_string());
    db.record_audit_event_at(event, 123).unwrap();

    let list = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args(["audit", "list", "--project", "audit-project", "--json"])
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list_stdout = String::from_utf8(list.stdout).unwrap();
    assert!(list_stdout.contains("security.blocked"));
    assert!(list_stdout.contains("<path>"));
    assert!(!list_stdout.contains(repo.to_str().unwrap()));
    assert!(!list_stdout.contains(home.to_str().unwrap()));
    assert!(!list_stdout.contains("secret-token"));

    let export = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "audit",
            "export",
            "--project",
            "audit-project",
            "--out",
            out.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        export.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&export.stderr)
    );
    let exported: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(exported["path"], out.to_str().unwrap());
    assert_eq!(exported["event_count"], 1);
    let raw_export = std::fs::read_to_string(&out).unwrap();
    assert!(raw_export.contains("security.blocked"));
    assert!(!raw_export.contains(repo.to_str().unwrap()));
    assert!(!raw_export.contains(home.to_str().unwrap()));
    assert!(!raw_export.contains("secret-token"));

    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn metrics_export_is_local_and_redacted_by_default() {
    use devrelay_core::{
        AuditEventInput, AuditEventType, AuditOutcome, DevRelayHome, MetadataDb, TaskRunInput,
        TaskRunState,
    };

    let home = std::env::temp_dir().join(format!("devrelay-metrics-test-{}", std::process::id()));
    let repo = home.join("repo");
    let out = home.join("metrics-export.json");
    let snapshot_out = home.join("snapshot.json");
    let bad_snapshot = home.join("bad-snapshot.json");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    write_manifest(&repo, "metrics-project", "Metrics Project");
    git(&repo, &["add", "devrelay.toml"]);
    git(&repo, &["commit", "-m", "manifest"]);

    let add = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "project",
            "add",
            repo.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );

    let checkpoint = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "checkpoint",
            "--repo",
            repo.to_str().unwrap(),
            "--manifest",
            repo.join("devrelay.toml").to_str().unwrap(),
            "--out",
            snapshot_out.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        checkpoint.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&checkpoint.stderr)
    );
    let checkpoint_json: serde_json::Value = serde_json::from_slice(&checkpoint.stdout).unwrap();
    let snapshot_repo = checkpoint_json["snapshot_repo"]
        .as_str()
        .unwrap()
        .to_string();
    let mut snapshot_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&snapshot_out).unwrap()).unwrap();
    snapshot_json["state_hash"] = serde_json::json!("bad-state-hash");
    std::fs::write(
        &bad_snapshot,
        serde_json::to_vec_pretty(&snapshot_json).unwrap(),
    )
    .unwrap();

    let failed_apply = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--json-errors",
            "--direct",
            "apply",
            "--repo",
            repo.to_str().unwrap(),
            "--source",
            &snapshot_repo,
            "--snapshot",
            bad_snapshot.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        !failed_apply.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&failed_apply.stdout),
        String::from_utf8_lossy(&failed_apply.stderr)
    );
    assert!(
        String::from_utf8_lossy(&failed_apply.stderr).contains("DR-APPLY-VERIFICATION-MISMATCH")
    );

    let devrelay_home = DevRelayHome::new(&home);
    let db = MetadataDb::open(devrelay_home.metadata_db_path("metrics-project")).unwrap();
    let mut checkpoint_failure = AuditEventInput::new(
        AuditEventType::SnapshotPublished,
        AuditOutcome::Failed,
        format!(
            "checkpoint failed at {} token=secret-token",
            repo.to_str().unwrap()
        ),
    );
    checkpoint_failure.project_id = Some("metrics-project".to_string());
    db.record_audit_event_at(checkpoint_failure, 123).unwrap();
    db.record_task_run_at(
        TaskRunInput {
            task_run_id: "tr_metrics".to_string(),
            project_id: "metrics-project".to_string(),
            session_id: None,
            state: TaskRunState::Succeeded,
            command: Some("cargo test".to_string()),
            metadata: serde_json::json!({
                "scheduler_reason": "local-cache-warm",
                "scheduler_explanation": format!("selected {}", repo.display()),
            }),
        },
        125,
        126,
    )
    .unwrap();

    let export = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "metrics",
            "export",
            "--project",
            "metrics-project",
            "--out",
            out.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        export.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&export.stderr)
    );
    let exported: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(exported["path"], out.to_str().unwrap());
    assert_eq!(exported["include_sensitive_paths"], false);
    assert_eq!(exported["source_code_included"], false);
    assert_eq!(exported["snapshot_objects_included"], false);
    assert_eq!(exported["report"]["privacy"]["local_by_default"], true);
    assert_eq!(exported["report"]["privacy"]["redacted"], true);
    assert_eq!(exported["report"]["checkpoints"]["successes"], 1);
    assert_eq!(exported["report"]["apply"]["verification_failures"], 1);
    assert_eq!(
        exported["report"]["scheduler"]["task_runs_with_choice_reason"],
        1
    );

    let raw_export = std::fs::read_to_string(&out).unwrap();
    assert!(raw_export.contains("metrics-project"));
    assert!(raw_export.contains("<path>"));
    assert!(raw_export.contains("<redacted>"));
    assert!(!raw_export.contains(repo.to_str().unwrap()));
    assert!(!raw_export.contains(home.to_str().unwrap()));
    assert!(!raw_export.contains("secret-token"));

    let _ = std::fs::remove_dir_all(home);
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

#[cfg(unix)]
struct RunningCliAgent {
    root: std::path::PathBuf,
    socket: std::path::PathBuf,
    child: std::process::Child,
}

#[cfg(unix)]
impl RunningCliAgent {
    fn start(name: &str) -> Self {
        use std::os::unix::fs::FileTypeExt;
        use std::process::Stdio;
        use std::time::{Duration, Instant};

        let root = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let config = root.join("config.toml");
        let socket = root.join("agent.sock");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();

        let mut child = devrelay_agent()
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
impl Drop for RunningCliAgent {
    fn drop(&mut self) {
        self.stop();
    }
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
fn agent_unavailable_reports_direct_fallback() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-agent-unavailable-test-{}-{}",
        std::process::id(),
        "repo"
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    init_git_repo(&root);
    write_manifest(
        &root,
        "agent-unavailable-project",
        "Agent Unavailable Project",
    );

    let output = devrelay()
        .args([
            "--json-errors",
            "--agent-socket",
            root.join("missing.sock").to_str().unwrap(),
            "status",
            "--repo",
            root.to_str().unwrap(),
            "--manifest",
            root.join("devrelay.toml").to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("DR-IPC-LOCAL"));
    assert!(stderr.contains("failed to contact local DevRelay agent"));
    assert!(stderr.contains("--direct"));

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn cli_project_commands_work_with_spawned_agent() {
    let mut running = RunningCliAgent::start("devrelay-cli-spawned-agent-test");
    let repo = running.root.join("spawned-agent-project");
    std::fs::create_dir(&repo).unwrap();
    init_git_repo(&repo);
    write_manifest(&repo, "spawned-agent-project", "Spawned Agent Project");
    git(&repo, &["add", "devrelay.toml"]);
    git(&repo, &["commit", "-m", "manifest"]);

    let add = devrelay()
        .env("DEVRELAY_HOME", &running.root)
        .args([
            "--agent-socket",
            running.socket.to_str().unwrap(),
            "project",
            "add",
            repo.to_str().unwrap(),
            "--manifest",
            repo.join("devrelay.toml").to_str().unwrap(),
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
        Some("spawned-agent-project")
    );

    let list = devrelay()
        .env("DEVRELAY_HOME", &running.root)
        .args([
            "--agent-socket",
            running.socket.to_str().unwrap(),
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
    assert_eq!(
        list_json[0]["display_name"].as_str(),
        Some("Spawned Agent Project")
    );
    assert!(running.root.join("config.toml").exists());

    running.stop();
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
    assert!(stdout.contains("\"mdns_enabled\": true"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn doctor_resources_reports_effective_policy() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-doctor-resources-test-{}",
        std::process::id()
    ));
    let config_path = root.join("devrelay.local.toml");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let config = devrelay_core::LocalConfig {
        resource_profile: devrelay_core::ResourceProfile::Custom,
        resource_policy_limits: None,
        ..devrelay_core::LocalConfig::new_for_local_device()
    };
    std::fs::write(&config_path, config.to_toml_string().unwrap()).unwrap();

    let output = devrelay()
        .env("DEVRELAY_HOME", root.join("home"))
        .args([
            "doctor",
            "resources",
            "--config",
            config_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["configured_profile"], "custom");
    assert_eq!(value["effective_profile"], "custom");
    assert_eq!(value["custom_limits_configured"], false);
    assert!(value["limits"]["cpu_slot_limit"].as_u64().unwrap() >= 1);
    assert!(
        value["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| { warning["code"] == "custom-limits-missing" })
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn doctor_anchor_health_reports_uninitialized_anchor() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-doctor-anchor-health-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);

    let output = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["doctor", "anchor-health", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["initialized"], false);
    assert_eq!(value["role"], "local-only");
    assert!(
        value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| { check["name"] == "anchor-role" && check["ok"] == false })
    );
    assert!(value["issues"].as_array().unwrap().iter().any(|issue| {
        issue["code"] == "anchor-not-configured"
            && issue["safe_actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| action.as_str().unwrap().contains("devrelay anchor init"))
    }));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn doctor_device_trust_reports_missing_paired_devices() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-doctor-device-trust-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);

    let output = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["doctor", "device-trust", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["device_count"], 1);
    assert_eq!(value["paired_device_count"], 0);
    assert_eq!(value["revoked_device_count"], 0);
    assert!(value["local_device_id"].as_str().unwrap().starts_with("d_"));
    assert!(value["issues"].as_array().unwrap().iter().any(|issue| {
        issue["code"] == "no-paired-devices"
            && issue["safe_actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| action.as_str().unwrap().contains("devrelay pairing start"))
    }));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anchor_init_and_status_report_layout() {
    let root = std::env::temp_dir().join(format!("devrelay-anchor-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let init = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["anchor", "init", "--json"])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&init.stderr)
    );
    let initialized: serde_json::Value = serde_json::from_slice(&init.stdout).unwrap();
    assert_eq!(initialized["initialized"], true);
    assert_eq!(initialized["role"], "anchor");
    assert_eq!(initialized["anchor_mode"], "user-selected");
    assert_eq!(
        initialized["layout"]["metadata_db_path"],
        root.join("anchor")
            .join("metadata.sqlite")
            .to_str()
            .unwrap()
    );
    assert!(root.join("config.toml").exists());
    assert!(root.join("anchor").join("metadata.sqlite").exists());
    assert!(root.join("anchor").join("snapshots").is_dir());
    assert!(root.join("anchor").join("cas").is_dir());
    assert!(root.join("anchor").join("startup.json").exists());

    let status = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["anchor", "status", "--json"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["initialized"], true);
    assert_eq!(status["role"], "anchor");
    assert_eq!(status["metadata_db_exists"], true);
    assert_eq!(status["startup_path_exists"], true);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn project_add_in_anchor_mode_creates_anchor_project_repo() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-anchor-project-add-test-{}",
        std::process::id()
    ));
    let repo = root.join("repo");
    let config = root.join("config.toml");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    write_manifest(&repo, "anchor-project", "Anchor Project");

    let init = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["anchor", "init", "--json"])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&init.stderr)
    );

    let add = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "project",
            "add",
            repo.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );

    let anchor_repo = root
        .join("anchor")
        .join("snapshots")
        .join("anchor-project.git");
    assert!(anchor_repo.join("HEAD").exists());
    let bare = Command::new("git")
        .arg("-C")
        .arg(&anchor_repo)
        .args(["rev-parse", "--is-bare-repository"])
        .output()
        .unwrap();
    assert!(bare.status.success());
    assert_eq!(String::from_utf8_lossy(&bare.stdout).trim(), "true");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anchor_maintenance_reports_known_snapshot_refs_and_runs_gc() {
    use devrelay_core::{
        AnchorSnapshotRepo, CanonicalPublishRequest, DevRelayHome, GitRepo, LeaseRecord,
        LeaseState, Manifest, MetadataDb, SnapshotStore,
    };

    let root = std::env::temp_dir().join(format!(
        "devrelay-anchor-maintenance-test-{}",
        std::process::id()
    ));
    let source_path = root.join("source");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&source_path).unwrap();
    init_git_repo(&source_path);
    write_manifest(&source_path, "maint-project", "Maintenance Project");
    git(&source_path, &["add", "devrelay.toml"]);
    git(&source_path, &["commit", "-m", "manifest"]);

    let home = DevRelayHome::new(&root);
    let manifest = Manifest::load(source_path.join("devrelay.toml")).unwrap();
    let source = GitRepo::new(&source_path);
    std::fs::write(source_path.join("README.md"), "changed\n").unwrap();
    let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();
    let stored = store.checkpoint(&source, &manifest, false, None).unwrap();
    let anchor = AnchorSnapshotRepo::open(&home, &manifest.project_id).unwrap();
    anchor
        .import_snapshot_from_store(&store, &stored.snapshot_id)
        .unwrap();

    let mut db = MetadataDb::open(home.anchor_metadata_db_path()).unwrap();
    let session = db
        .ensure_default_session(&manifest.project_id, &manifest.name, None)
        .unwrap();
    let lease = LeaseRecord {
        lease_id: "lease-maintenance".to_string(),
        project_id: manifest.project_id.clone(),
        session_id: session.session_id.clone(),
        state: LeaseState::Active,
        epoch: 1,
        holder_device_id: Some("device-a".to_string()),
        latest_snapshot_id: None,
        handoff_id: None,
    };
    db.upsert_lease(&lease).unwrap();
    let mut metadata = stored.metadata.clone();
    metadata.session_id = Some(session.session_id.clone());
    db.publish_snapshot_canonical(CanonicalPublishRequest {
        lease_id: &lease.lease_id,
        session_id: &session.session_id,
        expected_epoch: 1,
        holder_device_id: "device-a",
        expected_latest_snapshot_id: None,
        metadata: &metadata,
        pinned: false,
        label: Some("maintenance"),
    })
    .unwrap();

    let inspect = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "anchor",
            "maintenance",
            "--project",
            "maint-project",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspected: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inspected["project_id"], "maint-project");
    assert_eq!(inspected["known_snapshot_count"], 1);
    assert_eq!(inspected["known_snapshot_ids"][0], stored.snapshot_id);
    assert_eq!(
        inspected["report"]["orphan_refs"].as_array().unwrap().len(),
        0
    );
    assert_eq!(
        inspected["report"]["missing_refs"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(inspected["report"]["gc_ran"], false);

    let gc = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "anchor",
            "maintenance",
            "--project",
            "maint-project",
            "--gc",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        gc.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&gc.stderr)
    );
    let gc: serde_json::Value = serde_json::from_slice(&gc.stdout).unwrap();
    assert_eq!(gc["gc_requested"], true);
    assert_eq!(gc["report"]["gc_ran"], true);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn device_commands_show_generated_local_identity() {
    let root = std::env::temp_dir().join(format!("devrelay-device-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let list = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["devices", "list", "--json"])
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let devices: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let local = devices.as_array().unwrap().first().unwrap();
    let platform = devrelay_core::detect_platform_identity();
    let device_id = local["device_id"].as_str().unwrap();
    assert!(device_id.starts_with("d_"));
    assert!(!local["display_name"].as_str().unwrap().is_empty());
    assert_eq!(local["platform_key"], platform.platform_key);
    assert_eq!(local["architecture"], platform.architecture);
    assert!(
        serde_json::from_str::<serde_json::Value>(local["capabilities_json"].as_str().unwrap())
            .unwrap()
            .is_object()
    );
    assert!(local["paired_at_unix_seconds"].is_null());
    assert!(local["last_seen_unix_seconds"].as_u64().unwrap() > 0);
    assert!(root.join("config.toml").exists());
    assert!(root.join("agent.sqlite").exists());

    let show = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["device", "show", device_id, "--json"])
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&show.stderr)
    );
    let shown: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(shown["device_id"], device_id);
    assert_eq!(shown["display_name"], local["display_name"]);

    let revoke = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "device",
            "revoke",
            "d_peer_lost",
            "--reason",
            "lost laptop",
            "--key-rotation-required",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        revoke.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&revoke.stderr)
    );
    let revoked: serde_json::Value = serde_json::from_slice(&revoke.stdout).unwrap();
    assert_eq!(revoked["device_id"], "d_peer_lost");
    assert_eq!(revoked["revoked_by_device_id"], device_id);
    assert_eq!(revoked["reason"], "lost laptop");
    assert_eq!(revoked["key_rotation_required"], true);

    let audit = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["audit", "list", "--json"])
        .output()
        .unwrap();
    assert!(
        audit.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&audit.stderr)
    );
    let audit: serde_json::Value = serde_json::from_slice(&audit.stdout).unwrap();
    assert_eq!(audit[0]["type"], "device.revoked");
    assert_eq!(audit[0]["target_device_id"], "d_peer_lost");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn identity_commands_create_public_fabric_identity() {
    let root = std::env::temp_dir().join(format!("devrelay-identity-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let init = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["identity", "init", "--json"])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&init.stderr)
    );
    let init_json: serde_json::Value = serde_json::from_slice(&init.stdout).unwrap();
    let fabric_id = init_json["root"]["fabric_id"].as_str().unwrap();
    assert!(fabric_id.starts_with("f_"));
    assert_eq!(
        init_json["root"]["root_public_key_hex"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        init_json["device"]["fabric_id"].as_str().unwrap(),
        fabric_id
    );
    assert_eq!(
        init_json["device"]["signing_public_key_hex"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        init_json["device"]["network_public_key_hex"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(init_json["recovery_export"]["available"], false);
    assert!(
        root.join("identity")
            .join("dev-fabric-secret.json")
            .exists()
    );
    assert!(root.join("agent.sqlite").exists());

    let show = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["identity", "show", "--json"])
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&show.stderr)
    );
    let show_json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(show_json["root"]["fabric_id"].as_str().unwrap(), fabric_id);

    let export = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["identity", "recovery-export", "--json"])
        .output()
        .unwrap();
    assert!(
        export.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&export.stderr)
    );
    let export_json: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(export_json["available"], false);
    assert!(
        export_json["message"]
            .as_str()
            .unwrap()
            .contains("recovery export")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn discovery_advertise_dry_run_limits_txt_records() {
    let root = std::env::temp_dir().join(format!("devrelay-discovery-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let output = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "discovery",
            "advertise",
            "--role",
            "anchor",
            "--port",
            "7717",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["dry_run"].as_bool().unwrap());
    assert!(value["mdns_enabled"].as_bool().unwrap());
    assert!(!value["advertised"].as_bool().unwrap());
    let advertisement = &value["advertisement"];
    assert_eq!(
        advertisement["service_type"],
        "_devrelay-anchor._tcp.local."
    );
    assert_eq!(advertisement["port"], 7717);
    let txt = advertisement["txt"].as_object().unwrap();
    assert_eq!(txt.len(), 4);
    assert_eq!(txt["protocol"], "1");
    assert_eq!(txt["port"], "7717");
    assert!(txt["fabric"].as_str().unwrap().len() <= 12);
    assert!(txt["device_id"].as_str().unwrap().starts_with("d_"));
    assert!(advertisement.get("project").is_none());
    assert!(advertisement.get("path").is_none());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn discovery_browse_uses_manual_address_when_mdns_disabled() {
    let root = std::env::temp_dir().join(format!(
        "devrelay-discovery-manual-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("config.toml"),
        r#"
version = 1
fabric_name = "Personal Fabric"
device_id = "d_testmanual"
device_name = "test-device"
platform_key = "test"
architecture = "test"
capabilities_json = "{\"anchor\":true,\"local_snapshots\":true}"
paired_at_unix_seconds = 0
last_seen_unix_seconds = 0
resource_profile = "balanced"
anchor_mode = "local-only"
mdns_enabled = false
manual_discovery_address = "192.0.2.44:7717"

[editor]
command = "system"
"#,
    )
    .unwrap();

    let output = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["discovery", "browse", "--role", "anchor", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["role"], "anchor");
    assert_eq!(value["service_type"], "_devrelay-anchor._tcp.local.");
    assert!(!value["mdns_enabled"].as_bool().unwrap());
    assert_eq!(value["manual_address"], "192.0.2.44:7717");
    assert!(!value["browser_started"].as_bool().unwrap());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pairing_commands_start_confirm_and_abort() {
    let root = std::env::temp_dir().join(format!("devrelay-pairing-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let peer_signing = "b".repeat(64);
    let peer_network = "c".repeat(64);
    let peer_ephemeral = "d".repeat(64);

    let start = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "pairing",
            "start",
            "--peer-device-id",
            "d_peer",
            "--peer-name",
            "Peer Laptop",
            "--peer-signing-public-key",
        ])
        .arg(&peer_signing)
        .args(["--peer-network-public-key"])
        .arg(&peer_network)
        .args(["--peer-ephemeral-public-key"])
        .arg(&peer_ephemeral)
        .args(["--anchor", "192.0.2.1:7000", "--json"])
        .output()
        .unwrap();
    assert!(
        start.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&start.stderr)
    );
    let start_json: serde_json::Value = serde_json::from_slice(&start.stdout).unwrap();
    let pairing_id = start_json["pairing_id"].as_str().unwrap();
    let code = start_json["short_authentication_string"].as_str().unwrap();
    assert!(pairing_id.starts_with("pa_"));
    assert_eq!(start_json["state"], "pending");
    assert_eq!(start_json["anchor_address"], "192.0.2.1:7000");
    assert_eq!(code.len(), 6);

    let confirm = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["pairing", "confirm", pairing_id, "--code", code, "--json"])
        .output()
        .unwrap();
    assert!(
        confirm.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&confirm.stderr)
    );
    let confirm_json: serde_json::Value = serde_json::from_slice(&confirm.stdout).unwrap();
    assert_eq!(confirm_json["state"], "confirmed");
    let certificate: serde_json::Value =
        serde_json::from_str(confirm_json["certificate_json"].as_str().unwrap()).unwrap();
    assert_eq!(certificate["device_id"], "d_peer");
    assert!(
        certificate["signature_hex"]
            .as_str()
            .is_some_and(|value| value.len() == 128)
    );

    let start_abort = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args([
            "pairing",
            "start",
            "--peer-device-id",
            "d_peer_abort",
            "--peer-name",
            "Peer Abort",
            "--peer-signing-public-key",
        ])
        .arg(&peer_signing)
        .args(["--peer-network-public-key"])
        .arg(&peer_network)
        .args(["--peer-ephemeral-public-key"])
        .arg(&peer_ephemeral)
        .args(["--json"])
        .output()
        .unwrap();
    assert!(
        start_abort.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&start_abort.stderr)
    );
    let start_abort_json: serde_json::Value = serde_json::from_slice(&start_abort.stdout).unwrap();
    let abort_id = start_abort_json["pairing_id"].as_str().unwrap();
    let abort = devrelay()
        .env("DEVRELAY_HOME", &root)
        .args(["pairing", "abort", abort_id, "--json"])
        .output()
        .unwrap();
    assert!(
        abort.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&abort.stderr)
    );
    let abort_json: serde_json::Value = serde_json::from_slice(&abort.stdout).unwrap();
    assert_eq!(abort_json["state"], "aborted");

    let _ = std::fs::remove_dir_all(root);
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
fn session_commands_round_trip() {
    let root = std::env::temp_dir().join(format!("devrelay-session-test-{}", std::process::id()));
    let home = root.join("home");
    let repo = root.join("repo");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    write_manifest(&repo, "session-project", "Session Project");
    git(&repo, &["add", "devrelay.toml"]);
    git(&repo, &["commit", "-m", "manifest"]);

    let add = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "--direct",
            "project",
            "add",
            repo.to_str().unwrap(),
            "--manifest",
            repo.join("devrelay.toml").to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );

    let list = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args(["sessions", "list", "--project", "session-project", "--json"])
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let sessions: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let default = sessions.as_array().unwrap().first().unwrap();
    let session_id = default["session_id"].as_str().unwrap();
    assert!(session_id.starts_with("se_"));
    assert_eq!(default["project_id"], "session-project");
    assert_eq!(default["name"], "Session Project");
    assert_eq!(default["parent_session_id"], serde_json::Value::Null);
    assert_eq!(default["state"], "active");
    assert_eq!(default["archived_at_unix_seconds"], serde_json::Value::Null);

    let show = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args(["session", "show", session_id, "--json"])
        .output()
        .unwrap();
    assert!(show.status.success());
    let shown: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(shown["session_id"], session_id);

    let fork = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "session",
            "fork",
            session_id,
            "--project",
            "session-project",
            "--name",
            "Experiment",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        fork.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&fork.stderr)
    );
    let forked: serde_json::Value = serde_json::from_slice(&fork.stdout).unwrap();
    let fork_id = forked["session_id"].as_str().unwrap();
    assert!(fork_id.starts_with("se_"));
    assert_ne!(fork_id, session_id);
    assert_eq!(forked["name"], "Experiment");
    assert_eq!(forked["parent_session_id"], session_id);
    assert_eq!(forked["state"], "fork");

    let archive = devrelay()
        .env("DEVRELAY_HOME", &home)
        .args([
            "session",
            "archive",
            fork_id,
            "--project",
            "session-project",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        archive.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&archive.stderr)
    );
    let archived: serde_json::Value = serde_json::from_slice(&archive.stdout).unwrap();
    assert_eq!(archived["session_id"], fork_id);
    assert_eq!(archived["state"], "archived");
    assert!(archived["archived_at_unix_seconds"].as_u64().is_some());

    let _ = std::fs::remove_dir_all(root);
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
    assert_path_value(added["manifest_path"].as_str(), &canonical_manifest);
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
    assert!(
        workspace["device_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("d_"))
    );
    assert_path_value(
        workspace["local_path"].as_str(),
        &root.canonicalize().unwrap(),
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
    assert_path_value(
        show_json["local_path"].as_str(),
        &root_b.canonicalize().unwrap(),
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
fn safety_recovery_defaults_new_workspace_for_cli_recover_open() {
    // Invariant: safety/recovery_defaults_new_workspace.
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
    let source_readme = read_text_lf(root.join("README.md"));

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
    assert_eq!(open_json["path"].as_str(), Some(target.to_str().unwrap()));
    assert_ne!(open_json["path"].as_str(), Some(root.to_str().unwrap()));
    assert!(
        open_json["registered"]["workspace_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("w_"))
    );
    assert_eq!(read_text_lf(target.join("README.md")), "source changed\n");
    assert_eq!(read_text_lf(target.join("notes.md")), "recover me\n");
    assert_eq!(read_text_lf(root.join("README.md")), source_readme);

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
    assert!(
        apply_json["safe_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value
                .as_str()
                .is_some_and(|text| text.contains("separate work")))
    );
    let backup_snapshot_id = apply_json["backup"]["snapshot_id"].as_str().unwrap();
    assert_eq!(read_text_lf(target.join("README.md")), "source snapshot\n");

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
        read_text_lf(backup_recover.join("target-only.txt")),
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
        read_text_lf(applied_repo.join("README.md")),
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
    assert_eq!(read_text_lf(target.join("README.md")), "demo\n");

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
    assert_eq!(read_text_lf(target.join("README.md")), "handoff clean\n");

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
    assert_eq!(
        continued_json["workspace_states_updated"].as_bool(),
        Some(true)
    );
    let backup_snapshot_id = continued_json["backup"]["snapshot_id"].as_str().unwrap();
    assert_eq!(read_text_lf(target.join("README.md")), "handoff dirty\n");

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
        read_text_lf(backup_recover.join("target-only.txt")),
        "dirty target\n"
    );

    let project = devrelay()
        .args([
            "project",
            "show",
            "continue-dirty-project",
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
