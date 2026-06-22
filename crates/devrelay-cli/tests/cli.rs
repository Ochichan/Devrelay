use std::process::Command;

fn devrelay() -> Command {
    Command::new(env!("CARGO_BIN_EXE_devrelay"))
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
