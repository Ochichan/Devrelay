use devrelay_core::{
    DevRelayError, GitRepo, Manifest, SnapshotMetadata, VerificationDetails, apply_snapshot,
    classification_reason, create_snapshot, verify_snapshot,
};
use std::ffi::OsString;
use std::fs;
use std::path::Path;

fn manifest() -> Manifest {
    Manifest::parse(
        r#"
schema = 1
project_id = "12345678"
name = "roundtrip"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
    )
    .unwrap()
}

fn init_repo(path: &Path) -> GitRepo {
    fs::create_dir(path).unwrap();
    let repo = GitRepo::new(path);
    repo.run(&["init", "-b", "main"]).unwrap();
    repo.run(&["config", "user.name", "DevRelay Test"]).unwrap();
    repo.run(&["config", "user.email", "devrelay-test@example.local"])
        .unwrap();
    repo
}

fn write_base_files(path: &Path) {
    fs::write(path.join("tracked.txt"), "base\n").unwrap();
    fs::write(path.join("modify.txt"), "before\n").unwrap();
    fs::write(path.join("delete.txt"), "delete me\n").unwrap();
    fs::write(path.join("unstaged_delete.txt"), "delete me later\n").unwrap();
    fs::write(path.join("same_path.txt"), "old\n").unwrap();
    fs::write(path.join("rename_me.txt"), "rename me\n").unwrap();
    fs::write(path.join("binary.bin"), [0_u8, 1, 2, 3]).unwrap();
    fs::write(path.join("script.sh"), "#!/bin/sh\necho hi\n").unwrap();
    fs::write(path.join("유니코드-tracked.txt"), "unicode tracked base\n").unwrap();
}

fn commit_base(repo: &GitRepo, path: &Path) {
    write_base_files(path);
    repo.run(&["add", "."]).unwrap();
    repo.run(&["commit", "-m", "base"]).unwrap();
}

fn porcelain(repo: &GitRepo) -> String {
    repo.run(&[
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--untracked-files=all",
    ])
    .unwrap()
}

fn clone_target(source: &GitRepo, source_path: &Path, target_path: &Path) -> GitRepo {
    source
        .run_with_env(
            [
                OsString::from("clone"),
                source_path.as_os_str().to_os_string(),
                target_path.as_os_str().to_os_string(),
            ],
            &[],
        )
        .unwrap();
    GitRepo::new(target_path)
}

fn assert_source_unchanged(source: &GitRepo, before_status: &str, before_index_tree: &str) {
    assert_eq!(porcelain(source), before_status);
    assert_eq!(source.current_index_tree().unwrap(), before_index_tree);
}

fn assert_status_equivalent_after_apply(target: &GitRepo, snapshot: &SnapshotMetadata) {
    let status = target.status().unwrap();
    assert_eq!(status.counts.staged, snapshot.source_status.staged);
    assert_eq!(status.counts.unstaged, snapshot.source_status.unstaged);
    assert_eq!(status.counts.unmerged, 0);
    assert_eq!(status.counts.untracked, snapshot.included_untracked.len());
}

fn tree_mode(repo: &GitRepo, treeish: &str, path: &str) -> String {
    repo.run(&["ls-tree", treeish, "--", path])
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn tree_blob_content(repo: &GitRepo, treeish: &str, path: &str) -> String {
    let entry = repo.run(&["ls-tree", treeish, "--", path]).unwrap();
    let oid = entry.split_whitespace().nth(2).unwrap();
    repo.run(&["cat-file", "-p", oid]).unwrap()
}

fn index_mode(repo: &GitRepo, path: &str) -> String {
    repo.run(&["ls-files", "-s", "--", path])
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn assert_executable_mode_preserved(target: &GitRepo, snapshot: &SnapshotMetadata, path: &str) {
    assert_eq!(tree_mode(target, &snapshot.index_tree_oid, path), "100755");
    assert_eq!(tree_mode(target, &snapshot.work_tree_oid, path), "100755");
    assert_eq!(index_mode(target, path), "100755");
}

fn round_trip<F, A>(mutate: F, assert_target: A)
where
    F: FnOnce(&Path, &GitRepo),
    A: FnOnce(&Path, &GitRepo, &SnapshotMetadata, &VerificationDetails),
{
    let temp = tempfile::tempdir().unwrap();
    let source_path = temp.path().join("source");
    let target_path = temp.path().join("target");
    let source = init_repo(&source_path);
    commit_base(&source, &source_path);

    mutate(&source_path, &source);

    let before_status = porcelain(&source);
    let before_index_tree = source.current_index_tree().unwrap();
    let snapshot = create_snapshot(&source, &manifest()).unwrap();
    assert_source_unchanged(&source, &before_status, &before_index_tree);

    let target = clone_target(&source, &source_path, &target_path);
    let verification = apply_snapshot(&target, &source, &snapshot).unwrap();
    verify_snapshot(&target, &snapshot).unwrap();
    assert_status_equivalent_after_apply(&target, &snapshot);
    assert_target(&target_path, &target, &snapshot, &verification);
}

#[test]
fn round_trips_staged_delete_fixture() {
    round_trip(
        |_source_path, source| {
            source.run(&["rm", "delete.txt"]).unwrap();
        },
        |target_path, target, snapshot, _verification| {
            assert!(!target_path.join("delete.txt").exists());
            assert_eq!(target.status().unwrap().counts.staged, 1);
            assert_eq!(snapshot.source_status.staged, 1);
        },
    );
}

#[test]
fn round_trips_unstaged_delete_fixture() {
    round_trip(
        |source_path, _source| {
            fs::remove_file(source_path.join("unstaged_delete.txt")).unwrap();
        },
        |target_path, target, snapshot, _verification| {
            assert!(!target_path.join("unstaged_delete.txt").exists());
            assert_eq!(target.status().unwrap().counts.unstaged, 1);
            assert_eq!(snapshot.source_status.unstaged, 1);
        },
    );
}

#[test]
fn round_trips_staged_and_unstaged_tracked_changes() {
    round_trip(
        |source_path, source| {
            fs::write(source_path.join("staged_add.txt"), "new\n").unwrap();
            source.run(&["add", "staged_add.txt"]).unwrap();

            fs::write(source_path.join("modify.txt"), "staged modify\n").unwrap();
            source.run(&["add", "modify.txt"]).unwrap();

            source.run(&["rm", "delete.txt"]).unwrap();

            fs::write(source_path.join("tracked.txt"), "base\nunstaged\n").unwrap();
            fs::remove_file(source_path.join("unstaged_delete.txt")).unwrap();
        },
        |target_path, _target, _snapshot, _verification| {
            assert_eq!(
                fs::read_to_string(target_path.join("staged_add.txt")).unwrap(),
                "new\n"
            );
            assert_eq!(
                fs::read_to_string(target_path.join("modify.txt")).unwrap(),
                "staged modify\n"
            );
            assert!(!target_path.join("delete.txt").exists());
            assert_eq!(
                fs::read_to_string(target_path.join("tracked.txt")).unwrap(),
                "base\nunstaged\n"
            );
            assert!(!target_path.join("unstaged_delete.txt").exists());
        },
    );
}

#[test]
fn round_trips_untracked_paths_and_excludes_secret_and_generated_paths() {
    round_trip(
        |source_path, _source| {
            fs::write(source_path.join("notes.md"), "carry me\n").unwrap();
            fs::write(source_path.join("empty.txt"), "").unwrap();
            fs::write(source_path.join("유니코드.md"), "unicode\n").unwrap();
            fs::write(source_path.join("path with spaces.txt"), "spaces\n").unwrap();
            fs::write(source_path.join(".env"), "DATABASE_URL=secret\n").unwrap();
            fs::create_dir(source_path.join("target")).unwrap();
            fs::write(source_path.join("target/generated.bin"), "skip\n").unwrap();
        },
        |target_path, _target, snapshot, verification| {
            for path in [
                "notes.md",
                "empty.txt",
                "유니코드.md",
                "path with spaces.txt",
            ] {
                assert!(target_path.join(path).exists(), "{path} should be carried");
            }
            assert_eq!(fs::read(target_path.join("empty.txt")).unwrap(), b"");
            assert!(!target_path.join(".env").exists());
            assert!(!target_path.join("target/generated.bin").exists());
            assert!(snapshot.excluded.iter().any(|item| item.path == ".env"));
            assert!(
                snapshot
                    .excluded
                    .iter()
                    .any(|item| item.path == "target/generated.bin")
            );
            assert!(verification.excluded_paths.contains(&".env".to_string()));
        },
    );
}

#[test]
fn round_trips_unicode_tracked_paths() {
    round_trip(
        |source_path, source| {
            fs::write(
                source_path.join("유니코드-tracked.txt"),
                "unicode tracked changed\n",
            )
            .unwrap();
            source.run(&["add", "유니코드-tracked.txt"]).unwrap();
        },
        |target_path, target, snapshot, _verification| {
            assert_eq!(
                fs::read_to_string(target_path.join("유니코드-tracked.txt")).unwrap(),
                "unicode tracked changed\n"
            );
            assert_eq!(
                tree_blob_content(target, &snapshot.index_tree_oid, "유니코드-tracked.txt"),
                "unicode tracked changed"
            );
        },
    );
}

#[test]
fn round_trips_binary_file_modify_and_rename() {
    round_trip(
        |source_path, source| {
            fs::write(source_path.join("binary.bin"), [9_u8, 8, 7, 6, 5]).unwrap();
            source
                .run(&["mv", "rename_me.txt", "renamed file.txt"])
                .unwrap();
        },
        |target_path, _target, _snapshot, _verification| {
            assert_eq!(
                fs::read(target_path.join("binary.bin")).unwrap(),
                vec![9_u8, 8, 7, 6, 5]
            );
            assert!(target_path.join("renamed file.txt").exists());
            assert!(!target_path.join("rename_me.txt").exists());
        },
    );
}

#[cfg(unix)]
#[test]
fn round_trips_executable_bit_on_posix() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    round_trip(
        |source_path, source| {
            let script = source_path.join("script.sh");
            let mut permissions = fs::metadata(&script).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script, permissions).unwrap();
            source.run(&["add", "script.sh"]).unwrap();
        },
        |target_path, target, snapshot, _verification| {
            assert_executable_mode_preserved(target, snapshot, "script.sh");
            let mode = fs::metadata(target_path.join("script.sh")).unwrap().mode();
            assert_ne!(mode & 0o111, 0);
        },
    );
}

#[test]
fn round_trips_executable_index_mode_when_filemode_is_ignored() {
    round_trip(
        |_source_path, source| {
            source.run(&["config", "core.filemode", "false"]).unwrap();
            source
                .run(&["update-index", "--chmod=+x", "script.sh"])
                .unwrap();
        },
        |_target_path, target, snapshot, _verification| {
            assert_executable_mode_preserved(target, snapshot, "script.sh");
        },
    );
}

#[cfg(unix)]
#[test]
fn round_trips_symlink_target_string() {
    round_trip(
        |source_path, _source| {
            std::os::unix::fs::symlink("tracked.txt", source_path.join("tracked-link")).unwrap();
        },
        |target_path, target, snapshot, _verification| {
            assert_eq!(
                tree_mode(target, &snapshot.work_tree_oid, "tracked-link"),
                "120000"
            );
            assert_eq!(
                tree_blob_content(target, &snapshot.work_tree_oid, "tracked-link"),
                "tracked.txt"
            );
            let link_path = target_path.join("tracked-link");
            assert!(
                fs::symlink_metadata(&link_path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(fs::read_link(link_path).unwrap(), Path::new("tracked.txt"));
        },
    );
}

#[cfg(unix)]
#[test]
fn excludes_symlink_escape_without_following_target() {
    round_trip(
        |source_path, _source| {
            let outside = source_path.parent().unwrap().join("outside-private.pem");
            fs::write(outside, "-----BEGIN PRIVATE KEY-----\nsecret\n").unwrap();
            std::os::unix::fs::symlink("../outside-private.pem", source_path.join("outside-link"))
                .unwrap();
        },
        |target_path, _target, snapshot, _verification| {
            assert!(snapshot.excluded.iter().any(|item| {
                item.path == "outside-link"
                    && item.reason == classification_reason::SYMLINK_TARGET_OUTSIDE_WORKSPACE
            }));
            assert!(!target_path.join("outside-link").exists());
        },
    );
}

#[test]
fn round_trips_staged_delete_plus_same_path_recreation() {
    round_trip(
        |source_path, source| {
            source.run(&["rm", "same_path.txt"]).unwrap();
            fs::write(source_path.join("same_path.txt"), "new same path\n").unwrap();
        },
        |target_path, target, snapshot, _verification| {
            assert_eq!(
                fs::read_to_string(target_path.join("same_path.txt")).unwrap(),
                "new same path\n"
            );
            let status = target.status().unwrap();
            assert_eq!(status.counts.staged, snapshot.source_status.staged);
            assert_eq!(status.counts.untracked, snapshot.included_untracked.len());
        },
    );
}

#[test]
fn refuses_dirty_target_fixture() {
    let temp = tempfile::tempdir().unwrap();
    let source_path = temp.path().join("source");
    let target_path = temp.path().join("target");
    let source = init_repo(&source_path);
    commit_base(&source, &source_path);
    fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
    let snapshot = create_snapshot(&source, &manifest()).unwrap();
    let target = clone_target(&source, &source_path, &target_path);
    fs::write(target_path.join("local.txt"), "do not overwrite\n").unwrap();

    let err = apply_snapshot(&target, &source, &snapshot).unwrap_err();

    assert!(matches!(err, DevRelayError::TargetDirty(_)));
}
