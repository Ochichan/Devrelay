//! Git LFS pointer and local object availability inspection.

use crate::{
    CasChunkHash, CasStore, DevRelayError, GitRepo, Result, SnapshotMetadata, SnapshotSidecar,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;

const LFS_POINTER_VERSION: &str = "https://git-lfs.github.com/spec/v1";
pub const LFS_LOCAL_OBJECT_SIDECAR_CLASSIFICATION: &str = "git-lfs-local-object";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LfsObjectReport {
    pub repo: PathBuf,
    pub pointers: Vec<LfsPointer>,
    pub missing_objects: Vec<LfsMissingObject>,
    pub local_only_objects: Vec<LfsLocalOnlyObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LfsPointer {
    pub path: String,
    pub oid_sha256: String,
    pub size: u64,
    pub local_object_present: bool,
    pub upstream_object_present: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LfsMissingObject {
    pub path: String,
    pub oid_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LfsLocalOnlyObject {
    pub path: String,
    pub oid_sha256: String,
}

pub fn inspect_lfs_objects(repo: &GitRepo) -> Result<LfsObjectReport> {
    inspect_lfs_objects_inner(repo, None)
}

pub fn inspect_lfs_objects_with_upstream(
    repo: &GitRepo,
    upstream_git_dir: impl AsRef<std::path::Path>,
) -> Result<LfsObjectReport> {
    inspect_lfs_objects_inner(repo, Some(upstream_git_dir.as_ref()))
}

fn inspect_lfs_objects_inner(
    repo: &GitRepo,
    upstream_git_dir: Option<&std::path::Path>,
) -> Result<LfsObjectReport> {
    let git_dir = repo.git_dir()?;
    let mut pointers = Vec::new();
    for path in tracked_paths(repo)? {
        let Some(pointer) = parse_lfs_pointer_from_worktree(repo, &path)? else {
            continue;
        };
        pointers.push(lfs_pointer_report(
            path,
            pointer,
            &git_dir,
            upstream_git_dir,
        )?);
    }
    Ok(report_for_pointers(repo, pointers))
}

pub fn ensure_lfs_objects_available(repo: &GitRepo) -> Result<()> {
    let report = inspect_lfs_objects(repo)?;
    ensure_lfs_report_objects_available(&report)
}

pub fn ensure_lfs_report_objects_available(report: &LfsObjectReport) -> Result<()> {
    if report.missing_objects.is_empty() {
        return Ok(());
    }
    let missing = report
        .missing_objects
        .iter()
        .map(|object| format!("{} ({})", object.path, object.oid_sha256))
        .collect::<Vec<_>>()
        .join(", ");
    Err(DevRelayError::UnsupportedRepositoryState(format!(
        "missing or invalid Git LFS objects required for handoff: {missing}"
    )))
}

pub fn capture_local_only_lfs_objects(
    repo: &GitRepo,
    report: &LfsObjectReport,
    cas_store: &CasStore,
    chunk_size_bytes: usize,
) -> Result<Vec<SnapshotSidecar>> {
    if chunk_size_bytes == 0 {
        return Err(DevRelayError::Config(
            "LFS sidecar chunk size must be positive".to_string(),
        ));
    }

    let git_dir = repo.git_dir()?;
    let mut captured_oids = BTreeSet::new();
    let mut sidecars = Vec::new();
    for object in &report.local_only_objects {
        if !captured_oids.insert(object.oid_sha256.clone()) {
            continue;
        }
        let pointer = report
            .pointers
            .iter()
            .find(|pointer| pointer.oid_sha256 == object.oid_sha256)
            .ok_or_else(|| {
                DevRelayError::Config(format!(
                    "LFS local-only object {} did not have a pointer report",
                    object.oid_sha256
                ))
            })?;
        if !verify_lfs_object(&git_dir, &object.oid_sha256, pointer.size)? {
            return Err(DevRelayError::MissingSourceObject(format!(
                "local-only Git LFS object {} disappeared before CAS capture",
                object.oid_sha256
            )));
        }
        let chunk_hashes = upload_lfs_object_chunks(
            &lfs_object_path(&git_dir, &object.oid_sha256),
            cas_store,
            chunk_size_bytes,
        )?;
        let manifest = cas_store.create_manifest(&chunk_hashes)?;
        sidecars.push(SnapshotSidecar {
            logical_path: lfs_object_sidecar_path(&object.oid_sha256)?,
            file_mode: "100644".to_string(),
            classification: LFS_LOCAL_OBJECT_SIDECAR_CLASSIFICATION.to_string(),
            size_bytes: pointer.size,
            chunk_size_bytes: chunk_size_bytes as u64,
            root_hash: manifest.manifest_id.clone(),
            cas_manifest_id: manifest.manifest_id,
        });
    }
    sidecars.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    Ok(sidecars)
}

pub fn inspect_snapshot_lfs_objects(
    repo: &GitRepo,
    snapshot: &SnapshotMetadata,
) -> Result<LfsObjectReport> {
    let git_dir = repo.git_dir()?;
    let mut pointers = lfs_pointers_in_tree(repo, &git_dir, &snapshot.index_tree_oid)?;
    pointers.extend(lfs_pointers_in_tree(
        repo,
        &git_dir,
        &snapshot.work_tree_oid,
    )?);
    Ok(report_for_pointers(repo, pointers))
}

pub fn ensure_snapshot_lfs_objects_available(
    repo: &GitRepo,
    snapshot: &SnapshotMetadata,
) -> Result<()> {
    ensure_snapshot_lfs_objects_available_inner(repo, snapshot, false)
}

pub fn ensure_snapshot_lfs_objects_available_or_sidecars(
    repo: &GitRepo,
    snapshot: &SnapshotMetadata,
) -> Result<()> {
    ensure_snapshot_lfs_objects_available_inner(repo, snapshot, true)
}

fn ensure_snapshot_lfs_objects_available_inner(
    repo: &GitRepo,
    snapshot: &SnapshotMetadata,
    allow_lfs_sidecars: bool,
) -> Result<()> {
    let report = inspect_snapshot_lfs_objects(repo, snapshot)?;
    let missing_objects = report
        .missing_objects
        .iter()
        .filter(|object| {
            !allow_lfs_sidecars || !snapshot_has_lfs_object_sidecar(snapshot, &object.oid_sha256)
        })
        .collect::<Vec<_>>();
    if missing_objects.is_empty() {
        return Ok(());
    }
    let missing = missing_objects
        .iter()
        .map(|object| format!("{} ({})", object.path, object.oid_sha256))
        .collect::<Vec<_>>()
        .join(", ");
    Err(DevRelayError::MissingSourceObject(format!(
        "missing or invalid Git LFS objects required by snapshot: {missing}"
    )))
}

pub fn snapshot_has_lfs_object_sidecar(snapshot: &SnapshotMetadata, oid_sha256: &str) -> bool {
    let Ok(logical_path) = lfs_object_sidecar_path(oid_sha256) else {
        return false;
    };
    snapshot.sidecars.iter().any(|sidecar| {
        sidecar.classification == LFS_LOCAL_OBJECT_SIDECAR_CLASSIFICATION
            && sidecar.logical_path == logical_path
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedLfsPointer {
    oid_sha256: String,
    size: u64,
}

fn tracked_paths(repo: &GitRepo) -> Result<Vec<String>> {
    Ok(repo
        .run(&["ls-files", "-z"])?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| path.replace('\\', "/"))
        .collect())
}

fn lfs_pointers_in_tree(
    repo: &GitRepo,
    git_dir: &std::path::Path,
    treeish: &str,
) -> Result<Vec<LfsPointer>> {
    let raw = repo.run(&["ls-tree", "-r", "-z", treeish])?;
    let mut pointers = Vec::new();
    for record in raw.split('\0').filter(|record| !record.is_empty()) {
        let Some((metadata, path)) = record.split_once('\t') else {
            return Err(DevRelayError::Config(format!(
                "unexpected git ls-tree record: {record:?}"
            )));
        };
        let mut fields = metadata.split_whitespace();
        let mode = fields.next().unwrap_or_default();
        let object_type = fields.next().unwrap_or_default();
        let oid = fields.next().unwrap_or_default();
        if object_type != "blob" || !matches!(mode, "100644" | "100755") {
            continue;
        }
        let Some(pointer) = parse_lfs_pointer_from_blob(repo, oid)? else {
            continue;
        };
        pointers.push(lfs_pointer_report(
            path.replace('\\', "/"),
            pointer,
            git_dir,
            None,
        )?);
    }
    Ok(pointers)
}

fn parse_lfs_pointer_from_worktree(repo: &GitRepo, path: &str) -> Result<Option<ParsedLfsPointer>> {
    let bytes = match fs::read(repo.path().join(PathBuf::from(path))) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    if bytes.len() > 1024 {
        return Ok(None);
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    parse_lfs_pointer(text)
}

fn parse_lfs_pointer_from_blob(repo: &GitRepo, oid: &str) -> Result<Option<ParsedLfsPointer>> {
    let size = repo
        .run(&["cat-file", "-s", oid])?
        .parse::<u64>()
        .map_err(|_| DevRelayError::Config(format!("invalid Git blob size for {oid}")))?;
    if size > 1024 {
        return Ok(None);
    }
    parse_lfs_pointer(&repo.run(&["cat-file", "-p", oid])?)
}

fn parse_lfs_pointer(text: &str) -> Result<Option<ParsedLfsPointer>> {
    let mut version_ok = false;
    let mut oid_sha256 = None;
    let mut size = None;
    for line in text.lines() {
        if let Some(version) = line.strip_prefix("version ") {
            version_ok = version == LFS_POINTER_VERSION;
        } else if let Some(oid) = line.strip_prefix("oid sha256:") {
            if oid.len() == 64 && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                oid_sha256 = Some(oid.to_ascii_lowercase());
            }
        } else if let Some(raw_size) = line.strip_prefix("size ") {
            size = Some(raw_size.parse::<u64>().map_err(|_| {
                DevRelayError::Config(format!("invalid Git LFS pointer size: {raw_size:?}"))
            })?);
        }
    }

    Ok(match (version_ok, oid_sha256, size) {
        (true, Some(oid_sha256), Some(size)) => Some(ParsedLfsPointer { oid_sha256, size }),
        _ => None,
    })
}

fn report_for_pointers(repo: &GitRepo, mut pointers: Vec<LfsPointer>) -> LfsObjectReport {
    pointers.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.oid_sha256.cmp(&right.oid_sha256))
    });
    pointers.dedup_by(|left, right| {
        left.path == right.path
            && left.oid_sha256 == right.oid_sha256
            && left.size == right.size
            && left.local_object_present == right.local_object_present
            && left.upstream_object_present == right.upstream_object_present
    });
    let missing_objects = pointers
        .iter()
        .filter(|pointer| !pointer.local_object_present)
        .map(|pointer| LfsMissingObject {
            path: pointer.path.clone(),
            oid_sha256: pointer.oid_sha256.clone(),
        })
        .collect();
    let local_only_objects = pointers
        .iter()
        .filter(|pointer| {
            pointer.local_object_present && pointer.upstream_object_present == Some(false)
        })
        .map(|pointer| LfsLocalOnlyObject {
            path: pointer.path.clone(),
            oid_sha256: pointer.oid_sha256.clone(),
        })
        .collect();
    LfsObjectReport {
        repo: repo.path().to_path_buf(),
        pointers,
        missing_objects,
        local_only_objects,
    }
}

fn lfs_pointer_report(
    path: String,
    pointer: ParsedLfsPointer,
    git_dir: &std::path::Path,
    upstream_git_dir: Option<&std::path::Path>,
) -> Result<LfsPointer> {
    let local_object_present = verify_lfs_object(git_dir, &pointer.oid_sha256, pointer.size)?;
    let upstream_object_present = upstream_git_dir
        .map(|git_dir| verify_lfs_object(git_dir, &pointer.oid_sha256, pointer.size))
        .transpose()?;
    Ok(LfsPointer {
        path,
        oid_sha256: pointer.oid_sha256,
        size: pointer.size,
        local_object_present,
        upstream_object_present,
    })
}

fn upload_lfs_object_chunks(
    path: &std::path::Path,
    cas_store: &CasStore,
    chunk_size_bytes: usize,
) -> Result<Vec<CasChunkHash>> {
    let mut file = File::open(path)?;
    let mut buffer = vec![0_u8; chunk_size_bytes];
    let mut hashes = Vec::new();

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        let hash = CasChunkHash::from_bytes(chunk);
        cas_store.upload_chunk(chunk, &hash)?;
        hashes.push(hash);
    }

    Ok(hashes)
}

fn verify_lfs_object(git_dir: &std::path::Path, oid_sha256: &str, size: u64) -> Result<bool> {
    let path = lfs_object_path(git_dir, oid_sha256);
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() != size {
        return Ok(false);
    }

    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_lower(&hasher.finalize()) == oid_sha256)
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn lfs_object_path(git_dir: &std::path::Path, oid_sha256: &str) -> PathBuf {
    git_dir
        .join("lfs")
        .join("objects")
        .join(&oid_sha256[..2])
        .join(&oid_sha256[2..4])
        .join(oid_sha256)
}

fn lfs_object_sidecar_path(oid_sha256: &str) -> Result<String> {
    if !is_lfs_oid(oid_sha256) {
        return Err(DevRelayError::Config(format!(
            "invalid Git LFS object id {oid_sha256}"
        )));
    }
    let oid = oid_sha256.to_ascii_lowercase();
    Ok(format!(
        ".git/lfs/objects/{}/{}/{}",
        &oid[..2],
        &oid[2..4],
        oid
    ))
}

fn is_lfs_oid(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn detects_lfs_pointers_and_local_object_availability() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        let object = b"fake lfs object";
        let oid = sha256_hex(object);
        fs::write(
            temp.path().join("asset.bin"),
            lfs_pointer(&oid, object.len() as u64),
        )
        .unwrap();
        git(temp.path(), &["add", "asset.bin"]);
        git(temp.path(), &["commit", "-m", "lfs pointer"]);

        let missing_report = inspect_lfs_objects(&repo).unwrap();

        assert_eq!(missing_report.pointers.len(), 1);
        assert_eq!(missing_report.pointers[0].path, "asset.bin");
        assert_eq!(missing_report.pointers[0].size, object.len() as u64);
        assert!(!missing_report.pointers[0].local_object_present);
        assert_eq!(missing_report.missing_objects.len(), 1);
        assert!(ensure_lfs_objects_available(&repo).is_err());

        let object_path = lfs_object_path(&repo.git_dir().unwrap(), &oid);
        fs::create_dir_all(object_path.parent().unwrap()).unwrap();
        fs::write(object_path, object).unwrap();

        let available_report = inspect_lfs_objects(&repo).unwrap();

        assert!(available_report.pointers[0].local_object_present);
        assert!(available_report.missing_objects.is_empty());
        ensure_lfs_objects_available(&repo).unwrap();
    }

    #[test]
    fn checks_lfs_upstream_availability() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        let upstream_git_dir = temp.path().join("upstream.git");
        fs::create_dir_all(&upstream_git_dir).unwrap();
        let object = b"local only lfs object";
        let oid = sha256_hex(object);
        fs::write(
            temp.path().join("asset.bin"),
            lfs_pointer(&oid, object.len() as u64),
        )
        .unwrap();
        git(temp.path(), &["add", "asset.bin"]);
        git(temp.path(), &["commit", "-m", "lfs pointer"]);
        write_lfs_object(&repo.git_dir().unwrap(), &oid, object);

        let local_only_report =
            inspect_lfs_objects_with_upstream(&repo, &upstream_git_dir).unwrap();

        assert_eq!(local_only_report.pointers.len(), 1);
        assert_eq!(
            local_only_report.pointers[0].upstream_object_present,
            Some(false)
        );
        assert_eq!(local_only_report.local_only_objects.len(), 1);
        assert_eq!(local_only_report.local_only_objects[0].oid_sha256, oid);

        write_lfs_object(&upstream_git_dir, &oid, object);

        let upstream_report = inspect_lfs_objects_with_upstream(&repo, &upstream_git_dir).unwrap();

        assert_eq!(
            upstream_report.pointers[0].upstream_object_present,
            Some(true)
        );
        assert!(upstream_report.local_only_objects.is_empty());
    }

    #[test]
    fn rejects_corrupt_lfs_object() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        let object = b"expected lfs object";
        let oid = sha256_hex(object);
        fs::write(
            temp.path().join("asset.bin"),
            lfs_pointer(&oid, object.len() as u64),
        )
        .unwrap();
        git(temp.path(), &["add", "asset.bin"]);
        git(temp.path(), &["commit", "-m", "lfs pointer"]);

        let object_path = lfs_object_path(&repo.git_dir().unwrap(), &oid);
        fs::create_dir_all(object_path.parent().unwrap()).unwrap();
        fs::write(object_path, b"corrupt lfs object").unwrap();

        let report = inspect_lfs_objects(&repo).unwrap();

        assert!(!report.pointers[0].local_object_present);
        assert_eq!(report.missing_objects[0].oid_sha256, oid);
        assert!(ensure_lfs_objects_available(&repo).is_err());
    }

    #[test]
    fn ignores_non_pointer_files() {
        let pointer = parse_lfs_pointer("regular content\n").unwrap();

        assert_eq!(pointer, None);
    }

    fn lfs_pointer(oid: &str, size: u64) -> String {
        format!("version {LFS_POINTER_VERSION}\noid sha256:{oid}\nsize {size}\n")
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex_lower(&Sha256::digest(bytes))
    }

    fn write_lfs_object(git_dir: &Path, oid: &str, bytes: &[u8]) {
        let object_path = lfs_object_path(git_dir, oid);
        fs::create_dir_all(object_path.parent().unwrap()).unwrap();
        fs::write(object_path, bytes).unwrap();
    }

    fn init_repo(root: &Path) -> GitRepo {
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "DevRelay Test"]);
        git(
            root,
            &["config", "user.email", "devrelay-test@example.local"],
        );
        GitRepo::new(root)
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "DevRelay Test")
            .env("GIT_AUTHOR_EMAIL", "devrelay-test@example.local")
            .env("GIT_COMMITTER_NAME", "DevRelay Test")
            .env("GIT_COMMITTER_EMAIL", "devrelay-test@example.local")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
