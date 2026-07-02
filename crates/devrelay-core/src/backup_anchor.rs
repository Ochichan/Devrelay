//! Backup anchor replication, verification, restore, and manual promotion.
//!
//! A backup generation replicates the anchor data set defined in
//! `docs/backup-anchor.md`: a consistent SQLite metadata copy taken with
//! `VACUUM INTO`, per-project snapshot bare repositories restricted to
//! `refs/devrelay/*`, and the anchor CAS root. Every generation carries a
//! manifest signed by the fabric root key over the metadata hash, Git ref
//! tips, CAS digest, and revocation freshness, so restore and promotion can
//! prove the generation is complete and untampered. Promotion is a manual,
//! audited operation that never transfers writer leases by itself.

use crate::{
    AuditEventInput, AuditEventType, AuditOutcome, DevRelayError, DevRelayHome,
    FabricIdentityStore, GitRepo, MetadataDb, Result,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const BACKUP_GENERATION_SCHEMA_VERSION: u32 = 1;
pub const BACKUP_MANIFEST_FILE_NAME: &str = "backup-manifest.json";
pub const BACKUP_METADATA_FILE_NAME: &str = "metadata.sqlite";
pub const BACKUP_GENERATION_ID_PREFIX: &str = "bg_";
pub const DEFAULT_MAX_PROMOTION_AGE_SECONDS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupRepositoryManifest {
    pub repository_name: String,
    pub ref_tips: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupAnchorManifest {
    pub schema_version: u32,
    pub generation_id: String,
    pub fabric_id: String,
    pub source_anchor_device_id: String,
    pub created_at_unix_seconds: u64,
    pub metadata_backup_blake3: String,
    pub metadata_backup_bytes: u64,
    pub repositories: Vec<BackupRepositoryManifest>,
    pub cas_file_count: u64,
    pub cas_byte_count: u64,
    pub cas_digest_blake3: String,
    pub revocation_count: u64,
    pub latest_revocation_at_unix_seconds: Option<u64>,
    pub root_public_key_hex: String,
    pub signature_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupAnchorGeneration {
    pub path: PathBuf,
    pub manifest: BackupAnchorManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupAnchorVerification {
    pub generation_id: String,
    pub metadata_backup_verified: bool,
    pub repositories_verified: usize,
    pub cas_files_verified: u64,
    pub signature_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupAnchorPromotion {
    pub generation_id: String,
    pub source_anchor_device_id: String,
    pub promoted_device_id: String,
    pub generation_age_seconds: u64,
    pub revocation_count: u64,
    pub audit_id: i64,
}

/// Creates one append-only backup generation under `backup_root`.
pub fn create_backup_anchor_generation(
    home: &DevRelayHome,
    identity: &FabricIdentityStore,
    source_anchor_device_id: &str,
    backup_root: &Path,
    now_unix_seconds: u64,
) -> Result<BackupAnchorGeneration> {
    let anchor_db_path = home.anchor_metadata_db_path();
    if !anchor_db_path.exists() {
        return Err(DevRelayError::Config(format!(
            "no anchor metadata database at {}; backup requires an anchor home",
            anchor_db_path.display()
        )));
    }

    let staging = backup_root.join(format!(".staging-{now_unix_seconds}"));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;

    // Consistent, WAL-aware metadata copy.
    let metadata_target = staging.join(BACKUP_METADATA_FILE_NAME);
    let db = MetadataDb::open(&anchor_db_path)?;
    db.connection()
        .execute(
            "VACUUM INTO ?1",
            [metadata_target.to_string_lossy().as_ref()],
        )
        .map_err(|err| {
            DevRelayError::Config(format!("failed to snapshot anchor metadata: {err}"))
        })?;
    let revocations = db.list_device_revocations()?;
    let metadata_backup_bytes = fs::metadata(&metadata_target)?.len();
    let metadata_backup_blake3 = hash_file(&metadata_target)?;

    // Snapshot bare repositories, restricted to the DevRelay ref namespace.
    let repos_target_root = staging.join("repos");
    fs::create_dir_all(&repos_target_root)?;
    let mut repositories = Vec::new();
    let repo_source_root = home.anchor_snapshot_repo_root();
    if repo_source_root.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(&repo_source_root)?
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|entry| entry.path().is_dir())
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            let target = repos_target_root.join(&name);
            let target_repo = GitRepo::new(&target);
            fs::create_dir_all(&target)?;
            target_repo.run(&["init", "--bare", "--quiet"])?;
            let source = entry.path();
            let fetch_result = target_repo.run(&[
                "fetch",
                "--quiet",
                source.to_string_lossy().as_ref(),
                "+refs/devrelay/*:refs/devrelay/*",
            ]);
            if let Err(err) = fetch_result {
                return Err(DevRelayError::Config(format!(
                    "failed to replicate snapshot repository {name}: {err}"
                )));
            }
            repositories.push(BackupRepositoryManifest {
                repository_name: name,
                ref_tips: ref_tips(&target_repo)?,
            });
        }
    }

    // CAS root copy plus content digest.
    let cas_target_root = staging.join("cas");
    let (cas_file_count, cas_byte_count) =
        copy_directory_recursive(&home.anchor_cas_root(), &cas_target_root)?;
    let cas_digest_blake3 = hash_directory(&cas_target_root)?;

    let latest_revocation_at_unix_seconds = revocations
        .iter()
        .map(|revocation| revocation.revoked_at_unix_seconds)
        .max();
    let bundle_seed = serde_json::json!({
        "metadata": metadata_backup_blake3,
        "cas": cas_digest_blake3,
        "created_at": now_unix_seconds,
    });
    let generation_id = format!(
        "{BACKUP_GENERATION_ID_PREFIX}{}",
        &blake3::hash(serde_json::to_string(&bundle_seed)?.as_bytes()).to_hex()[..24]
    );

    let fabric_id = identity
        .public_bundle_from_store(&crate::LocalConfig::default())?
        .root
        .fabric_id;
    let mut manifest = BackupAnchorManifest {
        schema_version: BACKUP_GENERATION_SCHEMA_VERSION,
        generation_id: generation_id.clone(),
        fabric_id,
        source_anchor_device_id: source_anchor_device_id.to_string(),
        created_at_unix_seconds: now_unix_seconds,
        metadata_backup_blake3,
        metadata_backup_bytes,
        repositories,
        cas_file_count,
        cas_byte_count,
        cas_digest_blake3,
        revocation_count: revocations.len() as u64,
        latest_revocation_at_unix_seconds,
        root_public_key_hex: String::new(),
        signature_hex: String::new(),
    };
    let (root_public_key_hex, signature_hex) =
        identity.sign_with_root(&signed_manifest_payload(&manifest)?)?;
    manifest.root_public_key_hex = root_public_key_hex;
    manifest.signature_hex = signature_hex;
    fs::write(
        staging.join(BACKUP_MANIFEST_FILE_NAME),
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;

    // Publish the generation atomically; partial staging never becomes a
    // promotion candidate.
    let generation_path = backup_root.join(&generation_id);
    if generation_path.exists() {
        return Err(DevRelayError::Config(format!(
            "backup generation {generation_id} already exists"
        )));
    }
    fs::rename(&staging, &generation_path)?;
    Ok(BackupAnchorGeneration {
        path: generation_path,
        manifest,
    })
}

/// Verifies a generation's contents against its signed manifest.
pub fn verify_backup_anchor_generation(generation_path: &Path) -> Result<BackupAnchorVerification> {
    let manifest = load_backup_manifest(generation_path)?;
    if manifest.schema_version != BACKUP_GENERATION_SCHEMA_VERSION {
        return Err(DevRelayError::Config(format!(
            "unsupported backup generation schema {}",
            manifest.schema_version
        )));
    }

    let payload = signed_manifest_payload(&manifest)?;
    verify_root_signature(
        &manifest.root_public_key_hex,
        &payload,
        &manifest.signature_hex,
    )?;

    let metadata_path = generation_path.join(BACKUP_METADATA_FILE_NAME);
    if hash_file(&metadata_path)? != manifest.metadata_backup_blake3 {
        return Err(DevRelayError::Config(
            "backup metadata database does not match the signed manifest".to_string(),
        ));
    }

    for repository in &manifest.repositories {
        let repo_path = generation_path
            .join("repos")
            .join(&repository.repository_name);
        let actual = ref_tips(&GitRepo::new(&repo_path))?;
        if actual != repository.ref_tips {
            return Err(DevRelayError::Config(format!(
                "backup repository {} refs do not match the signed manifest",
                repository.repository_name
            )));
        }
    }

    let cas_root = generation_path.join("cas");
    if hash_directory(&cas_root)? != manifest.cas_digest_blake3 {
        return Err(DevRelayError::Config(
            "backup CAS contents do not match the signed manifest".to_string(),
        ));
    }
    let (cas_files, _) = directory_stats(&cas_root)?;
    if cas_files != manifest.cas_file_count {
        return Err(DevRelayError::Config(
            "backup CAS file count does not match the signed manifest".to_string(),
        ));
    }

    Ok(BackupAnchorVerification {
        generation_id: manifest.generation_id,
        metadata_backup_verified: true,
        repositories_verified: manifest.repositories.len(),
        cas_files_verified: cas_files,
        signature_verified: true,
    })
}

/// Restores a verified generation into a fresh anchor home.
pub fn restore_backup_anchor_generation(
    generation_path: &Path,
    target_home: &DevRelayHome,
) -> Result<BackupAnchorVerification> {
    let verification = verify_backup_anchor_generation(generation_path)?;
    let target_db = target_home.anchor_metadata_db_path();
    if target_db.exists() {
        return Err(DevRelayError::Config(format!(
            "refusing to restore over existing anchor metadata at {}",
            target_db.display()
        )));
    }
    target_home.create_anchor_dirs()?;
    fs::copy(generation_path.join(BACKUP_METADATA_FILE_NAME), &target_db)?;
    copy_directory_recursive(
        &generation_path.join("repos"),
        &target_home.anchor_snapshot_repo_root(),
    )?;
    copy_directory_recursive(&generation_path.join("cas"), &target_home.anchor_cas_root())?;
    Ok(verification)
}

#[derive(Debug, Clone, Copy)]
pub struct BackupPromotionRequest<'a> {
    pub generation_path: &'a Path,
    pub target_home: &'a DevRelayHome,
    pub identity: &'a FabricIdentityStore,
    pub promoted_device_id: &'a str,
    pub operator_confirmation: &'a str,
    pub allow_stale_revocations: bool,
    pub max_age_seconds: u64,
    pub now_unix_seconds: u64,
}

/// Manually promotes a backup generation to a new primary anchor.
///
/// The operator confirmation must name the source and target anchors as
/// `<source-device-id>-><target-device-id>`. Promotion fails on stale
/// revocation state unless explicitly allowed, verifies and restores the
/// generation, and records a `backup.promoted` audit event in the restored
/// metadata database. Writer leases are not transferred.
pub fn promote_backup_anchor_generation(
    request: BackupPromotionRequest<'_>,
) -> Result<BackupAnchorPromotion> {
    let BackupPromotionRequest {
        generation_path,
        target_home,
        identity,
        promoted_device_id,
        operator_confirmation,
        allow_stale_revocations,
        max_age_seconds,
        now_unix_seconds,
    } = request;
    let manifest = load_backup_manifest(generation_path)?;
    let expected_confirmation =
        format!("{}->{promoted_device_id}", manifest.source_anchor_device_id);
    if operator_confirmation != expected_confirmation {
        return Err(DevRelayError::Config(format!(
            "promotion requires operator confirmation {expected_confirmation:?}, got {operator_confirmation:?}"
        )));
    }

    let local_root = identity
        .public_bundle_from_store(&crate::LocalConfig::default())?
        .root;
    if manifest.root_public_key_hex != local_root.root_public_key_hex {
        return Err(DevRelayError::Config(
            "backup generation was signed by a different fabric root".to_string(),
        ));
    }

    let generation_age_seconds = now_unix_seconds.saturating_sub(manifest.created_at_unix_seconds);
    if generation_age_seconds > max_age_seconds && !allow_stale_revocations {
        return Err(DevRelayError::Config(format!(
            "backup generation is {generation_age_seconds}s old; revocation state may be stale. \
             Re-run with an explicit stale-revocation override after checking device revocations"
        )));
    }

    restore_backup_anchor_generation(generation_path, target_home)?;

    let db = MetadataDb::open(target_home.anchor_metadata_db_path())?;
    let mut audit = AuditEventInput::new(
        AuditEventType::BackupPromoted,
        AuditOutcome::Succeeded,
        "backup anchor generation promoted to primary",
    )
    .with_detail(serde_json::json!({
        "generation_id": manifest.generation_id,
        "source_anchor_device_id": manifest.source_anchor_device_id,
        "promoted_device_id": promoted_device_id,
        "generation_age_seconds": generation_age_seconds,
        "revocation_count": manifest.revocation_count,
        "latest_revocation_at_unix_seconds": manifest.latest_revocation_at_unix_seconds,
        "allow_stale_revocations": allow_stale_revocations,
    }));
    audit.actor_device_id = Some(promoted_device_id.to_string());
    let record = db.record_audit_event(audit)?;

    Ok(BackupAnchorPromotion {
        generation_id: manifest.generation_id,
        source_anchor_device_id: manifest.source_anchor_device_id,
        promoted_device_id: promoted_device_id.to_string(),
        generation_age_seconds,
        revocation_count: manifest.revocation_count,
        audit_id: record.audit_id,
    })
}

pub fn load_backup_manifest(generation_path: &Path) -> Result<BackupAnchorManifest> {
    let path = generation_path.join(BACKUP_MANIFEST_FILE_NAME);
    let raw = fs::read_to_string(&path).map_err(|err| {
        DevRelayError::Config(format!("no backup manifest at {}: {err}", path.display()))
    })?;
    Ok(serde_json::from_str(&raw)?)
}

fn signed_manifest_payload(manifest: &BackupAnchorManifest) -> Result<Vec<u8>> {
    let mut unsigned = manifest.clone();
    unsigned.root_public_key_hex = String::new();
    unsigned.signature_hex = String::new();
    Ok(serde_json::to_vec(&unsigned)?)
}

fn verify_root_signature(
    root_public_key_hex: &str,
    payload: &[u8],
    signature_hex: &str,
) -> Result<()> {
    let key_bytes = decode_fixed_hex::<32>("root_public_key_hex", root_public_key_hex)?;
    let signature_bytes = decode_fixed_hex::<64>("signature_hex", signature_hex)?;
    let key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|err| DevRelayError::Config(format!("invalid fabric root key: {err}")))?;
    key.verify(payload, &Signature::from_bytes(&signature_bytes))
        .map_err(|err| {
            DevRelayError::Config(format!("backup manifest signature is invalid: {err}"))
        })
}

fn ref_tips(repo: &GitRepo) -> Result<BTreeMap<String, String>> {
    let output = repo.run(&[
        "for-each-ref",
        "--format=%(objectname) %(refname)",
        "refs/devrelay/",
    ])?;
    let mut tips = BTreeMap::new();
    for line in output.lines() {
        if let Some((oid, name)) = line.trim().split_once(' ') {
            tips.insert(name.to_string(), oid.to_string());
        }
    }
    Ok(tips)
}

fn copy_directory_recursive(source: &Path, target: &Path) -> Result<(u64, u64)> {
    fs::create_dir_all(target)?;
    let mut files = 0u64;
    let mut bytes = 0u64;
    if !source.is_dir() {
        return Ok((files, bytes));
    }
    let mut entries: Vec<_> = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let destination = target.join(entry.file_name());
        if path.is_dir() {
            let (child_files, child_bytes) = copy_directory_recursive(&path, &destination)?;
            files += child_files;
            bytes += child_bytes;
        } else {
            fs::copy(&path, &destination)?;
            files += 1;
            bytes += fs::metadata(&destination)?.len();
        }
    }
    Ok((files, bytes))
}

fn directory_stats(root: &Path) -> Result<(u64, u64)> {
    let mut files = 0u64;
    let mut bytes = 0u64;
    for path in sorted_files_recursive(root)? {
        files += 1;
        bytes += fs::metadata(&path)?.len();
    }
    Ok((files, bytes))
}

fn sorted_files_recursive(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.is_dir() {
        return Ok(files);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let mut entries: Vec<_> = fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn hash_directory(root: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    for path in sorted_files_recursive(root)? {
        let relative = path
            .strip_prefix(root)
            .map_err(|err| DevRelayError::Config(format!("path outside backup root: {err}")))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(b":");
        hasher.update(hash_file(&path)?.as_bytes());
        hasher.update(b"\n");
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn decode_fixed_hex<const N: usize>(field: &str, value: &str) -> Result<[u8; N]> {
    if value.len() != N * 2 {
        return Err(DevRelayError::Config(format!(
            "{field} must be {} hex characters",
            N * 2
        )));
    }
    let mut bytes = [0u8; N];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = char::from(chunk[0])
            .to_digit(16)
            .ok_or_else(|| DevRelayError::Config(format!("{field} contains non-hex characters")))?;
        let low = char::from(chunk[1])
            .to_digit(16)
            .ok_or_else(|| DevRelayError::Config(format!("{field} contains non-hex characters")))?;
        bytes[index] = ((high << 4) | low) as u8;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CasStore, LocalConfig};

    struct AnchorFixture {
        _temp: tempfile::TempDir,
        home: DevRelayHome,
        identity: FabricIdentityStore,
        backup_root: PathBuf,
    }

    fn anchor_fixture() -> AnchorFixture {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("anchor-home"));
        home.create_anchor_dirs().unwrap();
        let identity = FabricIdentityStore::new(home.clone());
        identity
            .open_or_create(&LocalConfig::new_for_local_device())
            .unwrap();

        let db = MetadataDb::open(home.anchor_metadata_db_path()).unwrap();
        db.ensure_default_session("project123", "Backup Project", None)
            .unwrap();
        let mut db = db;
        db.revoke_device("stale-device", "anchor", "lost", false)
            .unwrap();

        // One snapshot bare repo holding a devrelay ref.
        let source_repo_path = temp.path().join("source-repo");
        std::fs::create_dir_all(&source_repo_path).unwrap();
        let source = GitRepo::new(&source_repo_path);
        source.run(&["init", "-b", "main"]).unwrap();
        source.run(&["config", "user.name", "Backup Test"]).unwrap();
        source
            .run(&["config", "user.email", "backup@test.local"])
            .unwrap();
        std::fs::write(source_repo_path.join("file.txt"), "content\n").unwrap();
        source.run(&["add", "."]).unwrap();
        source.run(&["commit", "-m", "base"]).unwrap();
        let bare_path = home.anchor_snapshot_repo_root().join("project123.git");
        std::fs::create_dir_all(&bare_path).unwrap();
        let bare = GitRepo::new(&bare_path);
        bare.run(&["init", "--bare", "--quiet"]).unwrap();
        bare.run(&[
            "fetch",
            source_repo_path.to_string_lossy().as_ref(),
            "+HEAD:refs/devrelay/snapshots/s1/work",
        ])
        .unwrap();
        // A non-devrelay ref that must not replicate.
        bare.run(&[
            "fetch",
            source_repo_path.to_string_lossy().as_ref(),
            "+HEAD:refs/heads/should-not-copy",
        ])
        .unwrap();

        let cas = CasStore::open(home.anchor_cas_root()).unwrap();
        let chunk_bytes = b"sidecar chunk bytes";
        cas.upload_chunk(chunk_bytes, &crate::CasChunkHash::from_bytes(chunk_bytes))
            .unwrap();

        AnchorFixture {
            backup_root: temp.path().join("backups"),
            _temp: temp,
            home,
            identity,
        }
    }

    #[test]
    fn backup_generation_round_trips_through_verify_restore_and_promote() {
        let fixture = anchor_fixture();

        let generation = create_backup_anchor_generation(
            &fixture.home,
            &fixture.identity,
            "anchor-device",
            &fixture.backup_root,
            1_000,
        )
        .unwrap();
        assert!(
            generation
                .manifest
                .generation_id
                .starts_with(BACKUP_GENERATION_ID_PREFIX)
        );
        assert_eq!(generation.manifest.revocation_count, 1);
        assert_eq!(generation.manifest.repositories.len(), 1);
        let tips = &generation.manifest.repositories[0].ref_tips;
        assert!(tips.contains_key("refs/devrelay/snapshots/s1/work"));
        assert!(!tips.keys().any(|name| name.contains("should-not-copy")));

        let verification = verify_backup_anchor_generation(&generation.path).unwrap();
        assert!(verification.signature_verified);
        assert_eq!(verification.repositories_verified, 1);
        assert!(verification.cas_files_verified > 0);

        let restore_temp = tempfile::tempdir().unwrap();
        let target_home = DevRelayHome::new(restore_temp.path());
        let promotion = promote_backup_anchor_generation(BackupPromotionRequest {
            generation_path: &generation.path,
            target_home: &target_home,
            identity: &fixture.identity,
            promoted_device_id: "promoted-device",
            operator_confirmation: "anchor-device->promoted-device",
            allow_stale_revocations: false,
            max_age_seconds: DEFAULT_MAX_PROMOTION_AGE_SECONDS,
            now_unix_seconds: 2_000,
        })
        .unwrap();
        assert_eq!(promotion.generation_id, generation.manifest.generation_id);
        assert_eq!(promotion.revocation_count, 1);

        let restored = MetadataDb::open(target_home.anchor_metadata_db_path()).unwrap();
        let sessions = restored.list_sessions(Some("project123")).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(restored.list_device_revocations().unwrap().len(), 1);
        let audits = restored.list_audit_events(None, 10).unwrap();
        assert!(
            audits
                .iter()
                .any(|event| event.event_type == AuditEventType::BackupPromoted)
        );
        let restored_repo = GitRepo::new(
            target_home
                .anchor_snapshot_repo_root()
                .join("project123.git"),
        );
        let refs = ref_tips(&restored_repo).unwrap();
        assert!(refs.contains_key("refs/devrelay/snapshots/s1/work"));
    }

    #[test]
    fn verification_rejects_tampered_metadata_and_cas() {
        let fixture = anchor_fixture();
        let generation = create_backup_anchor_generation(
            &fixture.home,
            &fixture.identity,
            "anchor-device",
            &fixture.backup_root,
            1_000,
        )
        .unwrap();

        let metadata_path = generation.path.join(BACKUP_METADATA_FILE_NAME);
        let mut bytes = std::fs::read(&metadata_path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&metadata_path, &bytes).unwrap();
        let err = verify_backup_anchor_generation(&generation.path).unwrap_err();
        assert!(err.to_string().contains("metadata database"));

        let fixture = anchor_fixture();
        let generation = create_backup_anchor_generation(
            &fixture.home,
            &fixture.identity,
            "anchor-device",
            &fixture.backup_root,
            1_000,
        )
        .unwrap();
        let chunk = sorted_files_recursive(&generation.path.join("cas"))
            .unwrap()
            .into_iter()
            .find(|path| path.to_string_lossy().contains("chunks"))
            .expect("backup should contain a CAS chunk");
        std::fs::write(&chunk, b"tampered").unwrap();
        let err = verify_backup_anchor_generation(&generation.path).unwrap_err();
        assert!(err.to_string().contains("CAS contents"));
    }

    #[test]
    fn promotion_requires_confirmation_and_fresh_revocations() {
        let fixture = anchor_fixture();
        let generation = create_backup_anchor_generation(
            &fixture.home,
            &fixture.identity,
            "anchor-device",
            &fixture.backup_root,
            1_000,
        )
        .unwrap();

        let restore_temp = tempfile::tempdir().unwrap();
        let target_home = DevRelayHome::new(restore_temp.path());

        let wrong_confirmation = promote_backup_anchor_generation(BackupPromotionRequest {
            generation_path: &generation.path,
            target_home: &target_home,
            identity: &fixture.identity,
            promoted_device_id: "promoted-device",
            operator_confirmation: "wrong",
            allow_stale_revocations: false,
            max_age_seconds: DEFAULT_MAX_PROMOTION_AGE_SECONDS,
            now_unix_seconds: 2_000,
        })
        .unwrap_err();
        assert!(
            wrong_confirmation
                .to_string()
                .contains("operator confirmation")
        );

        let stale = promote_backup_anchor_generation(BackupPromotionRequest {
            generation_path: &generation.path,
            target_home: &target_home,
            identity: &fixture.identity,
            promoted_device_id: "promoted-device",
            operator_confirmation: "anchor-device->promoted-device",
            allow_stale_revocations: false,
            max_age_seconds: 60,
            now_unix_seconds: 1_000_000,
        })
        .unwrap_err();
        assert!(stale.to_string().contains("revocation state may be stale"));

        promote_backup_anchor_generation(BackupPromotionRequest {
            generation_path: &generation.path,
            target_home: &target_home,
            identity: &fixture.identity,
            promoted_device_id: "promoted-device",
            operator_confirmation: "anchor-device->promoted-device",
            allow_stale_revocations: true,
            max_age_seconds: 60,
            now_unix_seconds: 1_000_000,
        })
        .unwrap();
    }

    #[test]
    fn restore_refuses_existing_anchor_metadata() {
        let fixture = anchor_fixture();
        let generation = create_backup_anchor_generation(
            &fixture.home,
            &fixture.identity,
            "anchor-device",
            &fixture.backup_root,
            1_000,
        )
        .unwrap();

        let err = restore_backup_anchor_generation(&generation.path, &fixture.home).unwrap_err();

        assert!(err.to_string().contains("refusing to restore"));
    }
}
