//! Large sidecar capture into CAS.
//!
//! Snapshot creation can keep oversized untracked files out of Git refs while
//! preserving their bytes as bounded-memory CAS sidecars.

use crate::{
    CasChunkHash, CasManifest, CasStore, ClassifiedPath, DevRelayError, PathDecision, Result,
    SnapshotSidecar, classification_reason,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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

pub fn ensure_sidecars_available(
    repo_root: &Path,
    sidecars: &[SnapshotSidecar],
    cas_store: &CasStore,
) -> Result<()> {
    for sidecar in sidecars {
        let target_path = repo_relative_path(repo_root, &sidecar.logical_path)?;
        ensure_materialization_path_safe(repo_root, &target_path)?;
        let manifest = fetch_sidecar_manifest(sidecar, cas_store)?;
        let hashes = manifest
            .chunks
            .iter()
            .map(|chunk| chunk.hash.clone())
            .collect::<Vec<_>>();
        let missing = cas_store.missing_chunks(&hashes);
        if !missing.is_empty() {
            return Err(DevRelayError::MissingSourceObject(format!(
                "missing {} CAS chunks for sidecar {}",
                missing.len(),
                sidecar.logical_path
            )));
        }
    }
    Ok(())
}

pub fn materialize_sidecars(
    repo_root: &Path,
    sidecars: &[SnapshotSidecar],
    cas_store: &CasStore,
) -> Result<()> {
    for sidecar in sidecars {
        let manifest = fetch_sidecar_manifest(sidecar, cas_store)?;
        materialize_one_sidecar(repo_root, sidecar, &manifest, cas_store)?;
    }
    Ok(())
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

fn fetch_sidecar_manifest(sidecar: &SnapshotSidecar, cas_store: &CasStore) -> Result<CasManifest> {
    sidecar.validate()?;
    let manifest = cas_store.fetch_manifest(&sidecar.cas_manifest_id)?;
    if manifest.manifest_id != sidecar.root_hash {
        return Err(DevRelayError::Verification(format!(
            "sidecar {} root hash mismatch: expected {}, got {}",
            sidecar.logical_path, sidecar.root_hash, manifest.manifest_id
        )));
    }
    if manifest.total_bytes != sidecar.size_bytes {
        return Err(DevRelayError::Verification(format!(
            "sidecar {} size mismatch: expected {}, got {}",
            sidecar.logical_path, sidecar.size_bytes, manifest.total_bytes
        )));
    }
    Ok(manifest)
}

fn materialize_one_sidecar(
    repo_root: &Path,
    sidecar: &SnapshotSidecar,
    manifest: &CasManifest,
    cas_store: &CasStore,
) -> Result<()> {
    let target_path = repo_relative_path(repo_root, &sidecar.logical_path)?;
    ensure_materialization_path_safe(repo_root, &target_path)?;
    let parent = target_path.parent().ok_or_else(|| {
        DevRelayError::Config(format!(
            "sidecar {} does not have a parent directory",
            sidecar.logical_path
        ))
    })?;
    fs::create_dir_all(parent)?;
    ensure_materialization_path_safe(repo_root, &target_path)?;

    let tmp = parent.join(format!(
        ".devrelay-sidecar-{}-{}.tmp",
        std::process::id(),
        unix_nanos()
    ));
    let write_result = (|| -> Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        for chunk in &manifest.chunks {
            if chunk.size_bytes > sidecar.chunk_size_bytes {
                return Err(DevRelayError::Verification(format!(
                    "sidecar {} chunk {} exceeds declared chunk size",
                    sidecar.logical_path,
                    chunk.hash.as_str()
                )));
            }
            let bytes = cas_store.download_chunk(&chunk.hash)?;
            if bytes.len() as u64 != chunk.size_bytes {
                return Err(DevRelayError::Verification(format!(
                    "sidecar {} chunk {} size mismatch",
                    sidecar.logical_path,
                    chunk.hash.as_str()
                )));
            }
            file.write_all(&bytes)?;
        }
        file.sync_all()?;
        fs::rename(&tmp, &target_path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result?;
    restore_file_mode(&target_path, &sidecar.file_mode)?;
    verify_materialized_sidecar(&target_path, sidecar, manifest)
}

fn ensure_materialization_path_safe(repo_root: &Path, target_path: &Path) -> Result<()> {
    let original_repo_root = repo_root;
    let canonical_repo_root = repo_root.canonicalize()?;
    let parent = target_path.parent().unwrap_or(repo_root);
    if parent.exists() {
        let parent_canonical = parent.canonicalize()?;
        if !parent_canonical.starts_with(&canonical_repo_root) {
            return Err(DevRelayError::Config(format!(
                "sidecar path {} escapes repository root {}",
                target_path.display(),
                canonical_repo_root.display()
            )));
        }
    }

    let relative = target_path.strip_prefix(original_repo_root).map_err(|_| {
        DevRelayError::Config(format!(
            "sidecar path {} must stay inside repository root {}",
            target_path.display(),
            original_repo_root.display()
        ))
    })?;
    let mut current = original_repo_root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                current.push(part);
                if let Ok(metadata) = fs::symlink_metadata(&current)
                    && metadata.file_type().is_symlink()
                {
                    return Err(DevRelayError::Config(format!(
                        "sidecar path {} crosses symlink {}",
                        target_path.display(),
                        current.display()
                    )));
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(DevRelayError::Config(format!(
                    "sidecar path {} must stay inside repository root",
                    target_path.display()
                )));
            }
        }
    }
    Ok(())
}

fn verify_materialized_sidecar(
    target_path: &Path,
    sidecar: &SnapshotSidecar,
    manifest: &CasManifest,
) -> Result<()> {
    let chunk_size = usize::try_from(sidecar.chunk_size_bytes).map_err(|_| {
        DevRelayError::Verification(format!(
            "sidecar {} chunk size does not fit this platform",
            sidecar.logical_path
        ))
    })?;
    let mut file = File::open(target_path)?;
    let mut buffer = vec![0_u8; chunk_size];
    let mut offset = 0_u64;
    let mut chunk_index = 0_usize;

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let Some(expected) = manifest.chunks.get(chunk_index) else {
            return Err(DevRelayError::Verification(format!(
                "sidecar {} materialized with extra chunks",
                sidecar.logical_path
            )));
        };
        let actual = CasChunkHash::from_bytes(&buffer[..read]);
        if actual != expected.hash {
            return Err(DevRelayError::Verification(format!(
                "sidecar {} materialized chunk hash mismatch at offset {}",
                sidecar.logical_path, offset
            )));
        }
        if expected.offset_bytes != offset || expected.size_bytes != read as u64 {
            return Err(DevRelayError::Verification(format!(
                "sidecar {} materialized chunk layout mismatch",
                sidecar.logical_path
            )));
        }
        offset = offset.saturating_add(read as u64);
        chunk_index += 1;
    }

    if chunk_index != manifest.chunks.len() || offset != sidecar.size_bytes {
        return Err(DevRelayError::Verification(format!(
            "sidecar {} materialized size mismatch",
            sidecar.logical_path
        )));
    }
    Ok(())
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

#[cfg(unix)]
fn restore_file_mode(path: &Path, file_mode: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = match file_mode {
        "100755" => 0o755,
        "100644" => 0o644,
        other => {
            return Err(DevRelayError::Config(format!(
                "unsupported sidecar file mode {other}"
            )));
        }
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn restore_file_mode(_path: &Path, _file_mode: &str) -> Result<()> {
    Ok(())
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
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

    #[cfg(unix)]
    #[test]
    fn materialization_rejects_symlink_parent_escape() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, repo.join("link")).unwrap();
        let cas = CasStore::open(temp.path().join("cas")).unwrap();
        let bytes = b"payload";
        let hash = CasChunkHash::from_bytes(bytes);
        cas.upload_chunk(bytes, &hash).unwrap();
        let manifest = cas.create_manifest(&[hash]).unwrap();
        let sidecar = SnapshotSidecar {
            logical_path: "link/file.bin".to_string(),
            file_mode: "100644".to_string(),
            classification: classification_reason::LARGE_FILE_THRESHOLD.to_string(),
            size_bytes: bytes.len() as u64,
            chunk_size_bytes: DEFAULT_SIDECAR_CHUNK_BYTES as u64,
            root_hash: manifest.manifest_id.clone(),
            cas_manifest_id: manifest.manifest_id,
        };

        let err = materialize_sidecars(&repo, &[sidecar], &cas).unwrap_err();

        assert!(err.to_string().contains("escapes") || err.to_string().contains("symlink"));
        assert!(!outside.join("file.bin").exists());
    }
}
