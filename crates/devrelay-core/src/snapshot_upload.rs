//! Snapshot data upload ordering for anchor-backed canonical publish.
//!
//! This module coordinates the data-plane side of publish before metadata is
//! allowed to advance canonical latest: mark upload pending, import Git refs,
//! copy sidecar CAS chunks, verify anchor availability, then publish metadata.

use crate::{
    AnchorSnapshotRepo, CanonicalPublishRequest, CanonicalPublishResult, CasChunkHash, CasStore,
    DevRelayError, GitRepo, MetadataDb, Result, SnapshotMetadata,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotDataUploadFaultPoint {
    AfterPendingMarker,
    AfterGitObjects,
    AfterCasObjects,
    AfterDataVerification,
}

impl SnapshotDataUploadFaultPoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::AfterPendingMarker => "after-pending-marker",
            Self::AfterGitObjects => "after-git-objects",
            Self::AfterCasObjects => "after-cas-objects",
            Self::AfterDataVerification => "after-data-verification",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SnapshotDataUpload<'a> {
    pub source_repo: &'a GitRepo,
    pub anchor_repo: &'a AnchorSnapshotRepo,
    pub source_cas: Option<&'a CasStore>,
    pub anchor_cas: Option<&'a CasStore>,
    pub fault: Option<SnapshotDataUploadFaultPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingSnapshotUpload {
    pub snapshot_id: String,
    pub project_id: String,
    pub index_ref: String,
    pub work_ref: String,
    pub cas_manifest_ids: Vec<String>,
    pub cas_reachability_root_ids: Vec<String>,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingSnapshotUploadCleanup {
    pub snapshot_id: String,
    pub removed_refs: Vec<String>,
    pub removed_cas_reachability_root_ids: Vec<String>,
    pub marker_removed: bool,
}

pub fn publish_snapshot_canonical_with_data(
    db: &mut MetadataDb,
    request: CanonicalPublishRequest<'_>,
    upload: SnapshotDataUpload<'_>,
) -> Result<CanonicalPublishResult> {
    request.metadata.validate()?;
    let snapshot_id = request.metadata.snapshot_id.clone();
    mark_snapshot_upload_pending(upload.anchor_repo, request.metadata)?;
    inject_fault(
        upload.fault,
        SnapshotDataUploadFaultPoint::AfterPendingMarker,
    )?;

    upload
        .anchor_repo
        .import_snapshot_from_repo(upload.source_repo, request.metadata)?;
    inject_fault(upload.fault, SnapshotDataUploadFaultPoint::AfterGitObjects)?;

    copy_snapshot_sidecars_to_anchor_cas(request.metadata, upload.source_cas, upload.anchor_cas)?;
    inject_fault(upload.fault, SnapshotDataUploadFaultPoint::AfterCasObjects)?;

    upload
        .anchor_repo
        .verify_snapshot_available(request.metadata)?;
    ensure_snapshot_sidecars_in_anchor_cas(request.metadata, upload.anchor_cas)?;
    inject_fault(
        upload.fault,
        SnapshotDataUploadFaultPoint::AfterDataVerification,
    )?;

    let result = db.publish_snapshot_canonical(request);
    if snapshot_metadata_exists(db, &snapshot_id)? {
        finish_snapshot_upload(upload.anchor_repo, &snapshot_id)?;
    }
    result
}

pub fn mark_snapshot_upload_pending(
    anchor_repo: &AnchorSnapshotRepo,
    metadata: &SnapshotMetadata,
) -> Result<PendingSnapshotUpload> {
    metadata.validate()?;
    if anchor_repo.project_id() != metadata.project_id {
        return Err(DevRelayError::Config(format!(
            "snapshot project_id {} does not match anchor project_id {}",
            metadata.project_id,
            anchor_repo.project_id()
        )));
    }
    let marker = PendingSnapshotUpload {
        snapshot_id: metadata.snapshot_id.clone(),
        project_id: metadata.project_id.clone(),
        index_ref: metadata.index_ref(),
        work_ref: metadata.work_ref(),
        cas_manifest_ids: metadata
            .sidecars
            .iter()
            .map(|sidecar| sidecar.cas_manifest_id.clone())
            .collect(),
        cas_reachability_root_ids: metadata
            .sidecars
            .iter()
            .enumerate()
            .map(|(index, _)| sidecar_reachability_root_id(&metadata.snapshot_id, index))
            .collect(),
        created_at_unix_seconds: unix_now_seconds(),
    };
    write_json_atomically(
        &pending_upload_marker_path(anchor_repo, &metadata.snapshot_id)?,
        &marker,
    )?;
    Ok(marker)
}

pub fn list_pending_snapshot_uploads(
    anchor_repo: &AnchorSnapshotRepo,
) -> Result<Vec<PendingSnapshotUpload>> {
    let dir = pending_upload_dir(anchor_repo);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut uploads: Vec<PendingSnapshotUpload> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("json")
        {
            let raw = fs::read_to_string(entry.path())?;
            uploads.push(serde_json::from_str(&raw)?);
        }
    }
    uploads.sort_by(|left, right| {
        left.created_at_unix_seconds
            .cmp(&right.created_at_unix_seconds)
            .then(left.snapshot_id.cmp(&right.snapshot_id))
    });
    Ok(uploads)
}

pub fn cleanup_pending_snapshot_upload(
    anchor_repo: &AnchorSnapshotRepo,
    anchor_cas: Option<&CasStore>,
    snapshot_id: &str,
) -> Result<Option<PendingSnapshotUploadCleanup>> {
    let marker_path = pending_upload_marker_path(anchor_repo, snapshot_id)?;
    if !marker_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&marker_path)?;
    let marker: PendingSnapshotUpload = serde_json::from_str(&raw)?;
    validate_pending_marker(anchor_repo, &marker)?;
    let repo = GitRepo::new(anchor_repo.repo_path());
    let mut removed_refs = Vec::new();
    for refname in [&marker.index_ref, &marker.work_ref] {
        repo.run(&["update-ref", "-d", refname])?;
        removed_refs.push(refname.clone());
    }

    let mut removed_cas_reachability_root_ids = Vec::new();
    if let Some(cas) = anchor_cas {
        for root_id in &marker.cas_reachability_root_ids {
            if cas.remove_reachability_root(root_id)? {
                removed_cas_reachability_root_ids.push(root_id.clone());
            }
        }
    }

    fs::remove_file(&marker_path)?;
    Ok(Some(PendingSnapshotUploadCleanup {
        snapshot_id: marker.snapshot_id,
        removed_refs,
        removed_cas_reachability_root_ids,
        marker_removed: true,
    }))
}

pub fn finish_snapshot_upload(anchor_repo: &AnchorSnapshotRepo, snapshot_id: &str) -> Result<bool> {
    let marker_path = pending_upload_marker_path(anchor_repo, snapshot_id)?;
    match fs::remove_file(marker_path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

fn copy_snapshot_sidecars_to_anchor_cas(
    metadata: &SnapshotMetadata,
    source_cas: Option<&CasStore>,
    anchor_cas: Option<&CasStore>,
) -> Result<()> {
    if metadata.sidecars.is_empty() {
        return Ok(());
    }
    let source_cas = source_cas.ok_or_else(|| {
        DevRelayError::Config("snapshot has sidecars but source CAS was not provided".to_string())
    })?;
    let anchor_cas = anchor_cas.ok_or_else(|| {
        DevRelayError::Config("snapshot has sidecars but anchor CAS was not provided".to_string())
    })?;

    for (sidecar_index, sidecar) in metadata.sidecars.iter().enumerate() {
        sidecar.validate()?;
        let manifest = source_cas.fetch_manifest(&sidecar.cas_manifest_id)?;
        if manifest.manifest_id != sidecar.root_hash || manifest.total_bytes != sidecar.size_bytes {
            return Err(DevRelayError::Verification(format!(
                "sidecar {} CAS manifest does not match snapshot metadata",
                sidecar.logical_path
            )));
        }

        let mut hashes = Vec::with_capacity(manifest.chunks.len());
        for chunk in &manifest.chunks {
            if chunk.size_bytes > sidecar.chunk_size_bytes {
                return Err(DevRelayError::Verification(format!(
                    "sidecar {} chunk {} exceeds declared chunk size",
                    sidecar.logical_path,
                    chunk.hash.as_str()
                )));
            }
            let bytes = source_cas.download_chunk(&chunk.hash)?;
            if bytes.len() as u64 != chunk.size_bytes {
                return Err(DevRelayError::Verification(format!(
                    "sidecar {} chunk {} size mismatch",
                    sidecar.logical_path,
                    chunk.hash.as_str()
                )));
            }
            anchor_cas.upload_chunk(&bytes, &chunk.hash)?;
            hashes.push(chunk.hash.clone());
        }
        let copied = anchor_cas.create_manifest(&hashes)?;
        if copied.manifest_id != manifest.manifest_id {
            return Err(DevRelayError::Verification(format!(
                "sidecar {} copied manifest id mismatch",
                sidecar.logical_path
            )));
        }
        anchor_cas.add_reachability_root(
            &sidecar_reachability_root_id(&metadata.snapshot_id, sidecar_index),
            &copied.manifest_id,
        )?;
    }
    Ok(())
}

fn ensure_snapshot_sidecars_in_anchor_cas(
    metadata: &SnapshotMetadata,
    anchor_cas: Option<&CasStore>,
) -> Result<()> {
    if metadata.sidecars.is_empty() {
        return Ok(());
    }
    let anchor_cas = anchor_cas.ok_or_else(|| {
        DevRelayError::Config("snapshot has sidecars but anchor CAS was not provided".to_string())
    })?;
    for sidecar in &metadata.sidecars {
        sidecar.validate()?;
        let manifest = anchor_cas.fetch_manifest(&sidecar.cas_manifest_id)?;
        if manifest.manifest_id != sidecar.root_hash || manifest.total_bytes != sidecar.size_bytes {
            return Err(DevRelayError::Verification(format!(
                "sidecar {} is not available in anchor CAS",
                sidecar.logical_path
            )));
        }
        let hashes = manifest
            .chunks
            .iter()
            .map(|chunk| chunk.hash.clone())
            .collect::<Vec<CasChunkHash>>();
        let missing = anchor_cas.missing_chunks(&hashes);
        if !missing.is_empty() {
            return Err(DevRelayError::MissingSourceObject(format!(
                "anchor CAS is missing {} chunks for sidecar {}",
                missing.len(),
                sidecar.logical_path
            )));
        }
    }
    Ok(())
}

fn validate_pending_marker(
    anchor_repo: &AnchorSnapshotRepo,
    marker: &PendingSnapshotUpload,
) -> Result<()> {
    validate_pending_snapshot_id(&marker.snapshot_id)?;
    if marker.project_id != anchor_repo.project_id() {
        return Err(DevRelayError::Config(format!(
            "pending upload project_id {} does not match anchor project_id {}",
            marker.project_id,
            anchor_repo.project_id()
        )));
    }
    let expected_index_ref = format!("refs/devrelay/snapshots/{}/index", marker.snapshot_id);
    let expected_work_ref = format!("refs/devrelay/snapshots/{}/work", marker.snapshot_id);
    if marker.index_ref != expected_index_ref || marker.work_ref != expected_work_ref {
        return Err(DevRelayError::Config(format!(
            "pending upload {} contains unexpected snapshot refs",
            marker.snapshot_id
        )));
    }
    Ok(())
}

fn snapshot_metadata_exists(db: &MetadataDb, snapshot_id: &str) -> Result<bool> {
    db.connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM snapshots WHERE snapshot_id = ?1)",
            [snapshot_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn pending_upload_dir(anchor_repo: &AnchorSnapshotRepo) -> PathBuf {
    anchor_repo.repo_path().join("devrelay-pending-uploads")
}

fn pending_upload_marker_path(
    anchor_repo: &AnchorSnapshotRepo,
    snapshot_id: &str,
) -> Result<PathBuf> {
    validate_pending_snapshot_id(snapshot_id)?;
    Ok(pending_upload_dir(anchor_repo).join(format!("{snapshot_id}.json")))
}

fn sidecar_reachability_root_id(snapshot_id: &str, sidecar_index: usize) -> String {
    format!("snapshot-upload-{snapshot_id}-{sidecar_index}")
}

fn validate_pending_snapshot_id(snapshot_id: &str) -> Result<()> {
    if snapshot_id.is_empty()
        || matches!(snapshot_id, "." | "..")
        || snapshot_id.contains('/')
        || snapshot_id.contains('\\')
        || snapshot_id.contains("..")
        || snapshot_id.bytes().any(|byte| byte <= 0x20 || byte == 0x7f)
    {
        return Err(DevRelayError::Config(format!(
            "snapshot upload id {snapshot_id} is not safe for a file path"
        )));
    }
    Ok(())
}

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        DevRelayError::Config(format!(
            "snapshot upload marker {} has no parent directory",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}-{}-{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("pending-upload"),
        std::process::id(),
        unix_nanos()
    ));
    let write_result = (|| -> Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result
}

fn inject_fault(
    configured: Option<SnapshotDataUploadFaultPoint>,
    fault: SnapshotDataUploadFaultPoint,
) -> Result<()> {
    if configured == Some(fault) {
        return Err(DevRelayError::Config(format!(
            "injected upload fault at {}",
            fault.as_str()
        )));
    }
    Ok(())
}

fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
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
    use crate::{
        DevRelayHome, LeaseRecord, LeaseState, Manifest, create_snapshot_with_sidecars,
        unix_now_seconds,
    };
    use std::fs;
    use std::path::Path;

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"
schema = 1
project_id = "upload-project"
name = "Upload Project"

[workspace]
untracked = "safe"
portable_paths = "strict"
large_file_threshold_mib = 1
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
        fs::write(path.join("tracked.txt"), "base\n").unwrap();
        repo.run(&["add", "."]).unwrap();
        repo.run(&["commit", "-m", "base"]).unwrap();
        repo
    }

    fn setup_db(home: &DevRelayHome) -> (MetadataDb, String, String) {
        let db = MetadataDb::open(home.anchor_metadata_db_path()).unwrap();
        let session = db
            .ensure_default_session("upload-project", "Upload Project", None)
            .unwrap();
        let lease = LeaseRecord {
            lease_id: "lease-upload".to_string(),
            project_id: "upload-project".to_string(),
            session_id: session.session_id.clone(),
            state: LeaseState::Active,
            epoch: 1,
            holder_device_id: Some("device-a".to_string()),
            latest_snapshot_id: None,
            handoff_id: None,
        };
        db.upsert_lease(&lease).unwrap();
        (db, session.session_id, lease.lease_id)
    }

    fn create_upload_snapshot(
        source: &GitRepo,
        source_path: &Path,
        source_cas: &CasStore,
        session_id: &str,
    ) -> SnapshotMetadata {
        fs::write(source_path.join("large.bin"), vec![7_u8; 1024 * 1024 + 17]).unwrap();
        let mut metadata = create_snapshot_with_sidecars(source, &manifest(), source_cas).unwrap();
        metadata.session_id = Some(session_id.to_string());
        metadata
    }

    fn request<'a>(
        lease_id: &'a str,
        session_id: &'a str,
        metadata: &'a SnapshotMetadata,
    ) -> CanonicalPublishRequest<'a> {
        CanonicalPublishRequest {
            lease_id,
            session_id,
            expected_epoch: 1,
            holder_device_id: "device-a",
            expected_latest_snapshot_id: None,
            metadata,
            pinned: false,
            label: Some("upload"),
        }
    }

    fn snapshots_for_project(db: &MetadataDb) -> Vec<String> {
        let mut statement = db
            .connection()
            .prepare(
                "SELECT snapshot_id FROM snapshots WHERE project_id = ?1 ORDER BY sequence_number",
            )
            .unwrap();
        let rows = statement
            .query_map(["upload-project"], |row| row.get::<_, String>(0))
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    }

    fn upload_context<'a>(
        source: &'a GitRepo,
        anchor: &'a AnchorSnapshotRepo,
        source_cas: &'a CasStore,
        anchor_cas: &'a CasStore,
        fault: Option<SnapshotDataUploadFaultPoint>,
    ) -> SnapshotDataUpload<'a> {
        SnapshotDataUpload {
            source_repo: source,
            anchor_repo: anchor,
            source_cas: Some(source_cas),
            anchor_cas: Some(anchor_cas),
            fault,
        }
    }

    #[test]
    fn guarded_publish_uploads_git_and_cas_before_advancing_latest() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let source_cas = CasStore::open(temp.path().join("source-cas")).unwrap();
        let anchor_cas = CasStore::open(home.anchor_cas_root()).unwrap();
        let anchor = AnchorSnapshotRepo::open(&home, "upload-project").unwrap();
        let (mut db, session_id, lease_id) = setup_db(&home);
        let metadata = create_upload_snapshot(&source, &source_path, &source_cas, &session_id);

        let result = publish_snapshot_canonical_with_data(
            &mut db,
            request(&lease_id, &session_id, &metadata),
            upload_context(&source, &anchor, &source_cas, &anchor_cas, None),
        )
        .unwrap();

        assert_eq!(result.latest_snapshot_id, metadata.snapshot_id);
        assert_eq!(
            db.get_lease(&lease_id)
                .unwrap()
                .unwrap()
                .latest_snapshot_id
                .as_deref(),
            Some(metadata.snapshot_id.as_str())
        );
        assert!(list_pending_snapshot_uploads(&anchor).unwrap().is_empty());
        anchor.verify_snapshot_available(&metadata).unwrap();
        ensure_snapshot_sidecars_in_anchor_cas(&metadata, Some(&anchor_cas)).unwrap();
    }

    #[test]
    fn network_cut_before_metadata_publish_does_not_update_latest() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let source_cas = CasStore::open(temp.path().join("source-cas")).unwrap();
        let anchor_cas = CasStore::open(home.anchor_cas_root()).unwrap();
        let anchor = AnchorSnapshotRepo::open(&home, "upload-project").unwrap();
        let (mut db, session_id, lease_id) = setup_db(&home);
        let metadata = create_upload_snapshot(&source, &source_path, &source_cas, &session_id);

        let err = publish_snapshot_canonical_with_data(
            &mut db,
            request(&lease_id, &session_id, &metadata),
            upload_context(
                &source,
                &anchor,
                &source_cas,
                &anchor_cas,
                Some(SnapshotDataUploadFaultPoint::AfterCasObjects),
            ),
        )
        .unwrap_err();

        assert!(err.to_string().contains("after-cas-objects"));
        assert!(snapshots_for_project(&db).is_empty());
        assert_eq!(
            db.get_lease(&lease_id).unwrap().unwrap().latest_snapshot_id,
            None
        );
        assert_eq!(list_pending_snapshot_uploads(&anchor).unwrap().len(), 1);
        anchor.verify_snapshot_available(&metadata).unwrap();
        ensure_snapshot_sidecars_in_anchor_cas(&metadata, Some(&anchor_cas)).unwrap();
    }

    #[test]
    fn retry_after_partial_upload_publishes_and_clears_pending_marker() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let source_cas = CasStore::open(temp.path().join("source-cas")).unwrap();
        let anchor_cas = CasStore::open(home.anchor_cas_root()).unwrap();
        let anchor = AnchorSnapshotRepo::open(&home, "upload-project").unwrap();
        let (mut db, session_id, lease_id) = setup_db(&home);
        let metadata = create_upload_snapshot(&source, &source_path, &source_cas, &session_id);

        let first = publish_snapshot_canonical_with_data(
            &mut db,
            request(&lease_id, &session_id, &metadata),
            upload_context(
                &source,
                &anchor,
                &source_cas,
                &anchor_cas,
                Some(SnapshotDataUploadFaultPoint::AfterGitObjects),
            ),
        )
        .unwrap_err();
        assert!(first.to_string().contains("after-git-objects"));
        assert_eq!(list_pending_snapshot_uploads(&anchor).unwrap().len(), 1);

        let result = publish_snapshot_canonical_with_data(
            &mut db,
            request(&lease_id, &session_id, &metadata),
            upload_context(&source, &anchor, &source_cas, &anchor_cas, None),
        )
        .unwrap();

        assert_eq!(result.latest_snapshot_id, metadata.snapshot_id);
        assert_eq!(
            snapshots_for_project(&db),
            vec![metadata.snapshot_id.clone()]
        );
        assert!(list_pending_snapshot_uploads(&anchor).unwrap().is_empty());
    }

    #[test]
    fn cleanup_pending_upload_removes_orphan_refs_roots_and_marker() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let source_cas = CasStore::open(temp.path().join("source-cas")).unwrap();
        let anchor_cas = CasStore::open(home.anchor_cas_root()).unwrap();
        let anchor = AnchorSnapshotRepo::open(&home, "upload-project").unwrap();
        let (mut db, session_id, lease_id) = setup_db(&home);
        let metadata = create_upload_snapshot(&source, &source_path, &source_cas, &session_id);

        publish_snapshot_canonical_with_data(
            &mut db,
            request(&lease_id, &session_id, &metadata),
            upload_context(
                &source,
                &anchor,
                &source_cas,
                &anchor_cas,
                Some(SnapshotDataUploadFaultPoint::AfterCasObjects),
            ),
        )
        .unwrap_err();
        let pending = list_pending_snapshot_uploads(&anchor).unwrap();
        assert_eq!(pending.len(), 1);
        for root_id in &pending[0].cas_reachability_root_ids {
            anchor_cas.fetch_reachability_root(root_id).unwrap();
        }

        let cleanup =
            cleanup_pending_snapshot_upload(&anchor, Some(&anchor_cas), &metadata.snapshot_id)
                .unwrap()
                .unwrap();

        assert!(cleanup.marker_removed);
        assert_eq!(cleanup.removed_refs.len(), 2);
        assert_eq!(cleanup.removed_cas_reachability_root_ids.len(), 1);
        assert!(list_pending_snapshot_uploads(&anchor).unwrap().is_empty());
        assert!(
            GitRepo::new(anchor.repo_path())
                .run(&["rev-parse", "--verify", &metadata.index_ref()])
                .is_err()
        );
        for root_id in cleanup.removed_cas_reachability_root_ids {
            assert!(anchor_cas.fetch_reachability_root(&root_id).is_err());
        }
        assert!(snapshots_for_project(&db).is_empty());
        assert_eq!(
            db.get_lease(&lease_id).unwrap().unwrap().latest_snapshot_id,
            None
        );
    }

    #[test]
    fn stale_metadata_publish_keeps_data_but_does_not_advance_latest() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let source_cas = CasStore::open(temp.path().join("source-cas")).unwrap();
        let anchor_cas = CasStore::open(home.anchor_cas_root()).unwrap();
        let anchor = AnchorSnapshotRepo::open(&home, "upload-project").unwrap();
        let (mut db, session_id, lease_id) = setup_db(&home);
        let metadata = create_upload_snapshot(&source, &source_path, &source_cas, &session_id);

        let mut lease = db.get_lease(&lease_id).unwrap().unwrap();
        lease.epoch = 2;
        db.upsert_lease(&lease).unwrap();
        let mut stale_request = request(&lease_id, &session_id, &metadata);
        stale_request.expected_epoch = 1;

        let err = publish_snapshot_canonical_with_data(
            &mut db,
            stale_request,
            upload_context(&source, &anchor, &source_cas, &anchor_cas, None),
        )
        .unwrap_err();

        assert!(err.to_string().contains("stale publish"));
        assert_eq!(
            snapshots_for_project(&db),
            vec![metadata.snapshot_id.clone()]
        );
        assert_eq!(
            db.get_lease(&lease_id).unwrap().unwrap().latest_snapshot_id,
            None
        );
        assert!(list_pending_snapshot_uploads(&anchor).unwrap().is_empty());
        assert!(unix_now_seconds() >= metadata.created_at_unix_seconds);
    }
}
