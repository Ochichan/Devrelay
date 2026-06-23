//! Large sidecar capture into CAS.
//!
//! Snapshot creation can keep oversized untracked files out of Git refs while
//! preserving their bytes as bounded-memory CAS sidecars.

use crate::{
    CasChunkHash, CasStore, ClassifiedPath, DevRelayError, PathDecision, Result, SnapshotSidecar,
    classification_reason,
};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const DEFAULT_SIDECAR_CHUNK_BYTES: usize = 1024 * 1024;

pub fn capture_large_sidecars(
    repo_root: &Path,
    classified: &[ClassifiedPath],
    cas_store: &CasStore,
    chunk_size_bytes: usize,
) -> Result<Vec<SnapshotSidecar>> {
    if chunk_size_bytes == 0 {
        return Err(DevRelayError::Config(
            "sidecar chunk size must be positive".to_string(),
        ));
    }

    let mut sidecars = Vec::new();
    for item in classified.iter().filter(|item| {
        item.decision == PathDecision::Exclude
            && item.reason == classification_reason::LARGE_FILE_THRESHOLD
    }) {
        let path = repo_relative_path(repo_root, &item.path)?;
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            continue;
        }
        let chunk_hashes = upload_file_chunks(&path, cas_store, chunk_size_bytes)?;
        let manifest = cas_store.create_manifest(&chunk_hashes)?;
        sidecars.push(SnapshotSidecar {
            logical_path: item.path.clone(),
            file_mode: file_mode(&metadata),
            classification: item.reason.clone(),
            size_bytes: metadata.len(),
            chunk_size_bytes: chunk_size_bytes as u64,
            root_hash: manifest.manifest_id.clone(),
            cas_manifest_id: manifest.manifest_id,
        });
    }
    sidecars.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    Ok(sidecars)
}

fn upload_file_chunks(
    path: &Path,
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

fn repo_relative_path(repo_root: &Path, logical_path: &str) -> Result<PathBuf> {
    let path = Path::new(logical_path);
    if path.is_absolute() {
        return Err(DevRelayError::Config(format!(
            "sidecar path {logical_path} must be repository-relative"
        )));
    }
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(DevRelayError::Config(format!(
                "sidecar path {logical_path} must not escape the repository"
            )));
        }
    }
    Ok(repo_root.join(path))
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o111 != 0 {
        "100755".to_string()
    } else {
        "100644".to_string()
    }
}

#[cfg(not(unix))]
fn file_mode(_metadata: &fs::Metadata) -> String {
    "100644".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_large_classified_file_into_fixed_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let cas = CasStore::open(temp.path().join("cas")).unwrap();
        let bytes = b"abcdefghijklmnopqrstu";
        fs::write(temp.path().join("large.bin"), bytes).unwrap();
        let classified = vec![ClassifiedPath {
            path: "large.bin".to_string(),
            decision: PathDecision::Exclude,
            reason: classification_reason::LARGE_FILE_THRESHOLD.to_string(),
        }];

        let sidecars = capture_large_sidecars(temp.path(), &classified, &cas, 8).unwrap();

        assert_eq!(sidecars.len(), 1);
        assert_eq!(sidecars[0].logical_path, "large.bin");
        assert_eq!(
            sidecars[0].classification,
            classification_reason::LARGE_FILE_THRESHOLD
        );
        assert_eq!(sidecars[0].size_bytes, bytes.len() as u64);
        assert_eq!(sidecars[0].chunk_size_bytes, 8);
        assert_eq!(sidecars[0].root_hash, sidecars[0].cas_manifest_id);

        let manifest = cas.fetch_manifest(&sidecars[0].cas_manifest_id).unwrap();
        assert_eq!(manifest.chunks.len(), 3);
        let mut reconstructed = Vec::new();
        for chunk in manifest.chunks {
            reconstructed.extend(cas.download_chunk(&chunk.hash).unwrap());
        }
        assert_eq!(reconstructed, bytes);
    }
}
