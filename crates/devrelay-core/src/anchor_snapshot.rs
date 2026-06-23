//! Anchor-side Git snapshot repository management.
//!
//! Anchor repositories are per-project bare Git repositories that cache only
//! DevRelay snapshot refs. Network handlers should use these helpers before
//! serving target fetches or accepting imported source snapshots.

use crate::{
    DEVRELAY_SNAPSHOT_REF_NAMESPACE, DevRelayError, DevRelayHome, GitDataPlaneAuthorization,
    GitDataPlaneAuthorizationRequest, GitDataPlaneOperation, GitDataPlanePolicy,
    GitDataPlaneRefSpec, GitRepo, GitRepositorySize, ProjectRegistryIndex, Result,
    SnapshotMetadata, SnapshotStore, StoredSnapshot, authorize_git_data_plane_project,
    ensure_git_object_available, inspect_git_repository_size, verify_git_repository_integrity,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct AnchorSnapshotRepo {
    project_id: String,
    repo_path: PathBuf,
    policy: GitDataPlanePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorSnapshotRef {
    pub snapshot_id: String,
    pub refname: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorSnapshotMaintenanceReport {
    pub orphan_refs: Vec<AnchorSnapshotRef>,
    pub missing_refs: Vec<AnchorSnapshotRef>,
    pub repository_size: GitRepositorySize,
    pub gc_ran: bool,
}

impl AnchorSnapshotRepo {
    pub fn open(home: &DevRelayHome, project_id: &str) -> Result<Self> {
        Self::open_with_policy(home, project_id, GitDataPlanePolicy::default())
    }

    pub fn open_existing(home: &DevRelayHome, project_id: &str) -> Result<Self> {
        Self::open_existing_with_policy(home, project_id, GitDataPlanePolicy::default())
    }

    pub fn open_with_policy(
        home: &DevRelayHome,
        project_id: &str,
        policy: GitDataPlanePolicy,
    ) -> Result<Self> {
        home.create_anchor_dirs()?;
        let repo_path = anchor_project_snapshot_repo_path(home, project_id)?;
        ensure_bare_repo(&repo_path)?;
        Ok(Self {
            project_id: project_id.to_string(),
            repo_path,
            policy,
        })
    }

    pub fn open_existing_with_policy(
        home: &DevRelayHome,
        project_id: &str,
        policy: GitDataPlanePolicy,
    ) -> Result<Self> {
        home.create_anchor_dirs()?;
        let repo_path = anchor_project_snapshot_repo_path(home, project_id)?;
        if !repo_path.join("HEAD").exists() {
            return Err(DevRelayError::Config(format!(
                "anchor snapshot repo for project {project_id} does not exist"
            )));
        }
        Ok(Self {
            project_id: project_id.to_string(),
            repo_path,
            policy,
        })
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    pub fn import_snapshot_from_store(
        &self,
        store: &SnapshotStore,
        snapshot_id: &str,
    ) -> Result<StoredSnapshot> {
        let stored = store.get_snapshot(snapshot_id)?;
        let source = GitRepo::new(store.snapshot_repo_path());
        self.import_snapshot_from_repo(&source, &stored.metadata)?;
        Ok(stored)
    }

    pub fn import_snapshot_from_repo(
        &self,
        source: &GitRepo,
        metadata: &SnapshotMetadata,
    ) -> Result<()> {
        self.validate_snapshot_metadata(metadata)?;
        let source_path = source.path().as_os_str().to_os_string();
        self.repo().run_with_env(
            [
                OsString::from("fetch"),
                source_path,
                OsString::from(format!("{}:{}", metadata.index_ref(), metadata.index_ref())),
                OsString::from(format!("{}:{}", metadata.work_ref(), metadata.work_ref())),
            ],
            &[],
        )?;
        self.ensure_snapshot_objects(metadata)?;
        self.validate_quota()?;
        Ok(())
    }

    pub fn import_snapshot_from_repo_authorized(
        &self,
        source: &GitRepo,
        metadata: &SnapshotMetadata,
        registry: &ProjectRegistryIndex,
        device_id: &str,
    ) -> Result<GitDataPlaneAuthorization> {
        let authorization =
            self.authorize_device(registry, device_id, GitDataPlaneOperation::PushSnapshot)?;
        self.import_snapshot_from_repo(source, metadata)?;
        Ok(authorization)
    }

    pub fn export_snapshot_to_repo(
        &self,
        target: &GitRepo,
        metadata: &SnapshotMetadata,
    ) -> Result<()> {
        self.validate_snapshot_metadata(metadata)?;
        self.ensure_snapshot_objects(metadata)?;
        let anchor_path = self.repo_path.as_os_str().to_os_string();
        target.run_with_env(
            [
                OsString::from("fetch"),
                anchor_path,
                OsString::from(format!("{}:{}", metadata.index_ref(), metadata.index_ref())),
                OsString::from(format!("{}:{}", metadata.work_ref(), metadata.work_ref())),
            ],
            &[],
        )?;
        Ok(())
    }

    pub fn export_snapshot_to_repo_authorized(
        &self,
        target: &GitRepo,
        metadata: &SnapshotMetadata,
        registry: &ProjectRegistryIndex,
        device_id: &str,
    ) -> Result<GitDataPlaneAuthorization> {
        let authorization =
            self.authorize_device(registry, device_id, GitDataPlaneOperation::FetchSnapshot)?;
        self.export_snapshot_to_repo(target, metadata)?;
        Ok(authorization)
    }

    pub fn verify_snapshot_available(&self, metadata: &SnapshotMetadata) -> Result<()> {
        self.validate_snapshot_metadata(metadata)?;
        self.ensure_snapshot_objects(metadata)
    }

    pub fn scan_orphan_snapshot_refs(
        &self,
        known_snapshot_ids: &[String],
    ) -> Result<Vec<AnchorSnapshotRef>> {
        let known = known_snapshot_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let refs = self.list_snapshot_refs()?;
        Ok(refs
            .into_iter()
            .filter_map(|refname| {
                let Some((snapshot_id, kind)) = parse_snapshot_ref(&refname) else {
                    return Some(AnchorSnapshotRef {
                        snapshot_id: String::new(),
                        refname,
                    });
                };
                if !known.contains(snapshot_id) || !matches!(kind, "index" | "work") {
                    Some(AnchorSnapshotRef {
                        snapshot_id: snapshot_id.to_string(),
                        refname,
                    })
                } else {
                    None
                }
            })
            .collect())
    }

    pub fn missing_snapshot_refs(
        &self,
        known_snapshot_ids: &[String],
    ) -> Result<Vec<AnchorSnapshotRef>> {
        let repo = self.repo();
        let mut missing = Vec::new();
        for snapshot_id in known_snapshot_ids {
            for refname in expected_snapshot_refs(snapshot_id) {
                if repo.run(&["rev-parse", "--verify", &refname]).is_err() {
                    missing.push(AnchorSnapshotRef {
                        snapshot_id: snapshot_id.clone(),
                        refname,
                    });
                }
            }
        }
        Ok(missing)
    }

    pub fn inspect_maintenance(
        &self,
        known_snapshot_ids: &[String],
    ) -> Result<AnchorSnapshotMaintenanceReport> {
        Ok(AnchorSnapshotMaintenanceReport {
            orphan_refs: self.scan_orphan_snapshot_refs(known_snapshot_ids)?,
            missing_refs: self.missing_snapshot_refs(known_snapshot_ids)?,
            repository_size: inspect_git_repository_size(&self.repo())?,
            gc_ran: false,
        })
    }

    pub fn run_guarded_gc(
        &self,
        known_snapshot_ids: &[String],
    ) -> Result<AnchorSnapshotMaintenanceReport> {
        let mut report = self.inspect_maintenance(known_snapshot_ids)?;
        if !report.orphan_refs.is_empty() {
            return Err(DevRelayError::Config(format!(
                "refusing anchor repo gc with {} orphan snapshot refs",
                report.orphan_refs.len()
            )));
        }
        if !report.missing_refs.is_empty() {
            return Err(DevRelayError::Config(format!(
                "refusing anchor repo gc with {} missing snapshot refs",
                report.missing_refs.len()
            )));
        }

        let repo = self.repo();
        verify_git_repository_integrity(&repo)?;
        self.policy
            .validate_repository_quota(&report.repository_size)?;
        repo.run(&["gc", "--prune=now"])?;
        verify_git_repository_integrity(&repo)?;
        report.repository_size = inspect_git_repository_size(&repo)?;
        report.gc_ran = true;
        Ok(report)
    }

    fn validate_snapshot_metadata(&self, metadata: &SnapshotMetadata) -> Result<()> {
        metadata.validate()?;
        if metadata.project_id != self.project_id {
            return Err(DevRelayError::Config(format!(
                "snapshot project_id {} does not match anchor project_id {}",
                metadata.project_id, self.project_id
            )));
        }
        self.policy.validate_fetch_refspec(&GitDataPlaneRefSpec {
            source: metadata.index_ref(),
            destination: metadata.index_ref(),
        })?;
        self.policy.validate_fetch_refspec(&GitDataPlaneRefSpec {
            source: metadata.work_ref(),
            destination: metadata.work_ref(),
        })?;
        self.policy
            .validate_snapshot_ref(&metadata.index_ref(), "snapshot index ref")?;
        self.policy
            .validate_snapshot_ref(&metadata.work_ref(), "snapshot work ref")?;
        Ok(())
    }

    fn ensure_snapshot_objects(&self, metadata: &SnapshotMetadata) -> Result<()> {
        let repo = self.repo();
        let index_oid = repo.run(&["rev-parse", "--verify", &metadata.index_ref()])?;
        let work_oid = repo.run(&["rev-parse", "--verify", &metadata.work_ref()])?;
        ensure_git_object_available(&repo, &index_oid, &self.policy)?;
        ensure_git_object_available(&repo, &work_oid, &self.policy)?;
        Ok(())
    }

    fn validate_quota(&self) -> Result<()> {
        let size = inspect_git_repository_size(&self.repo())?;
        self.policy.validate_repository_quota(&size)
    }

    fn authorize_device(
        &self,
        registry: &ProjectRegistryIndex,
        device_id: &str,
        operation: GitDataPlaneOperation,
    ) -> Result<GitDataPlaneAuthorization> {
        authorize_git_data_plane_project(
            registry,
            GitDataPlaneAuthorizationRequest {
                project_id: &self.project_id,
                device_id,
                operation,
            },
        )
    }

    fn list_snapshot_refs(&self) -> Result<Vec<String>> {
        let namespace = DEVRELAY_SNAPSHOT_REF_NAMESPACE.trim_end_matches('/');
        let raw = self
            .repo()
            .run(&["for-each-ref", "--format=%(refname)", namespace])?;
        Ok(raw
            .lines()
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect())
    }

    fn repo(&self) -> GitRepo {
        GitRepo::new(&self.repo_path)
    }
}

fn anchor_project_snapshot_repo_path(home: &DevRelayHome, project_id: &str) -> Result<PathBuf> {
    validate_anchor_project_id(project_id)?;
    Ok(home
        .anchor_snapshot_repo_root()
        .join(format!("{project_id}.git")))
}

fn validate_anchor_project_id(project_id: &str) -> Result<()> {
    if project_id.is_empty()
        || matches!(project_id, "." | "..")
        || project_id.contains('/')
        || project_id.contains('\\')
        || project_id.contains("..")
        || project_id.bytes().any(|byte| byte <= 0x20 || byte == 0x7f)
    {
        return Err(DevRelayError::Config(format!(
            "anchor project_id {project_id} is not safe for a repository path"
        )));
    }
    Ok(())
}

fn expected_snapshot_refs(snapshot_id: &str) -> [String; 2] {
    [
        format!("{DEVRELAY_SNAPSHOT_REF_NAMESPACE}{snapshot_id}/index"),
        format!("{DEVRELAY_SNAPSHOT_REF_NAMESPACE}{snapshot_id}/work"),
    ]
}

fn parse_snapshot_ref(refname: &str) -> Option<(&str, &str)> {
    let suffix = refname.strip_prefix(DEVRELAY_SNAPSHOT_REF_NAMESPACE)?;
    let mut parts = suffix.split('/');
    let snapshot_id = parts.next()?;
    let kind = parts.next()?;
    if snapshot_id.is_empty() || parts.next().is_some() {
        return None;
    }
    Some((snapshot_id, kind))
}

fn ensure_bare_repo(path: &Path) -> Result<()> {
    if path.join("HEAD").exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let output = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(DevRelayError::GitCommand {
            cwd: path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            args: format!("init --bare {}", path.display()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Manifest, ProjectRegistryEntry, ProjectRegistryIndex, SnapshotStore,
        WorkspaceRegistryEntry, WorkspaceState,
    };
    use std::collections::BTreeMap;
    use std::fs;

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"
schema = 1
project_id = "anchor-project"
name = "Anchor Project"

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
        fs::write(path.join("tracked.txt"), "base\n").unwrap();
        repo.run(&["add", "."]).unwrap();
        repo.run(&["commit", "-m", "base"]).unwrap();
        repo
    }

    fn authorized_registry(project_id: &str) -> ProjectRegistryIndex {
        ProjectRegistryIndex {
            projects: BTreeMap::from([(
                project_id.to_string(),
                ProjectRegistryEntry {
                    project_id: project_id.to_string(),
                    display_name: "Anchor Project".to_string(),
                    local_path: PathBuf::from("/tmp/anchor-project"),
                    manifest_path: None,
                    remote_url_fingerprint: None,
                    root_commit_fingerprint: None,
                    workspaces: BTreeMap::from([(
                        "ws-authorized".to_string(),
                        WorkspaceRegistryEntry {
                            workspace_id: "ws-authorized".to_string(),
                            project_id: project_id.to_string(),
                            device_id: "device-a".to_string(),
                            local_path: PathBuf::from("/tmp/anchor-project"),
                            platform_profile: "macos-arm64".to_string(),
                            state: WorkspaceState::Active,
                            last_seen_head: None,
                            last_checkpoint_id: None,
                        },
                    )]),
                },
            )]),
        }
    }

    #[test]
    fn anchor_repo_imports_snapshot_store_refs_and_exports_to_target() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        let source = init_repo(&source_path);
        let target = init_repo(&target_path);
        let manifest = manifest();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let stored = store.checkpoint(&source, &manifest, false, None).unwrap();
        let anchor = AnchorSnapshotRepo::open(&home, &manifest.project_id).unwrap();

        let imported = anchor
            .import_snapshot_from_store(&store, &stored.snapshot_id)
            .unwrap();

        assert_eq!(imported.snapshot_id, stored.snapshot_id);
        assert!(anchor.repo_path().join("HEAD").exists());
        assert!(
            GitRepo::new(anchor.repo_path())
                .run(&["rev-parse", "--verify", &stored.metadata.index_ref()])
                .is_ok()
        );

        anchor
            .export_snapshot_to_repo(&target, &stored.metadata)
            .unwrap();
        assert!(
            target
                .run(&["rev-parse", "--verify", &stored.metadata.work_ref()])
                .is_ok()
        );
    }

    #[test]
    fn anchor_repo_exports_snapshot_after_source_and_store_are_offline() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        let source = init_repo(&source_path);
        let target = init_repo(&target_path);
        let manifest = manifest();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let stored = store.checkpoint(&source, &manifest, false, None).unwrap();
        let store_repo_path = store.snapshot_repo_path().to_path_buf();
        let anchor = AnchorSnapshotRepo::open(&home, &manifest.project_id).unwrap();
        anchor
            .import_snapshot_from_store(&store, &stored.snapshot_id)
            .unwrap();
        drop(store);
        fs::remove_dir_all(&source_path).unwrap();
        fs::remove_dir_all(&store_repo_path).unwrap();

        anchor
            .export_snapshot_to_repo(&target, &stored.metadata)
            .unwrap();

        assert!(
            target
                .run(&["rev-parse", "--verify", &stored.metadata.index_ref()])
                .is_ok()
        );
        assert!(
            target
                .run(&["rev-parse", "--verify", &stored.metadata.work_ref()])
                .is_ok()
        );
    }

    #[test]
    fn anchor_repo_authorized_wrappers_gate_import_and_export_by_project_device() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        let source = init_repo(&source_path);
        let target = init_repo(&target_path);
        let manifest = manifest();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let stored = store.checkpoint(&source, &manifest, false, None).unwrap();
        let source_store_repo = GitRepo::new(store.snapshot_repo_path());
        let anchor = AnchorSnapshotRepo::open(&home, &manifest.project_id).unwrap();
        let registry = authorized_registry(&manifest.project_id);

        let rejected = anchor
            .import_snapshot_from_repo_authorized(
                &source_store_repo,
                &stored.metadata,
                &registry,
                "device-b",
            )
            .unwrap_err();
        assert!(rejected.to_string().contains("not authorized"));
        assert!(
            GitRepo::new(anchor.repo_path())
                .run(&["rev-parse", "--verify", &stored.metadata.index_ref()])
                .is_err()
        );

        let import_auth = anchor
            .import_snapshot_from_repo_authorized(
                &source_store_repo,
                &stored.metadata,
                &registry,
                "device-a",
            )
            .unwrap();
        assert_eq!(import_auth.workspace_ids, vec!["ws-authorized"]);

        let export_rejected = anchor
            .export_snapshot_to_repo_authorized(&target, &stored.metadata, &registry, "device-b")
            .unwrap_err();
        assert!(export_rejected.to_string().contains("not authorized"));
        assert!(
            target
                .run(&["rev-parse", "--verify", &stored.metadata.work_ref()])
                .is_err()
        );

        let export_auth = anchor
            .export_snapshot_to_repo_authorized(&target, &stored.metadata, &registry, "device-a")
            .unwrap();
        assert_eq!(export_auth.workspace_ids, vec!["ws-authorized"]);
        assert!(
            target
                .run(&["rev-parse", "--verify", &stored.metadata.work_ref()])
                .is_ok()
        );
    }

    #[test]
    fn anchor_repo_scans_orphans_and_guards_gc() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let manifest = manifest();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let stored = store.checkpoint(&source, &manifest, false, None).unwrap();
        let anchor = AnchorSnapshotRepo::open(&home, &manifest.project_id).unwrap();
        anchor
            .import_snapshot_from_store(&store, &stored.snapshot_id)
            .unwrap();

        let known = vec![stored.snapshot_id.clone()];
        let clean_report = anchor.run_guarded_gc(&known).unwrap();
        assert!(clean_report.gc_ran);
        assert!(clean_report.orphan_refs.is_empty());
        assert!(clean_report.missing_refs.is_empty());

        let anchor_repo = GitRepo::new(anchor.repo_path());
        let orphan_ref = "refs/devrelay/snapshots/s1_aaaaaaaaaaaaaaaaaaaaaaaa/metadata";
        anchor_repo
            .run(&["update-ref", orphan_ref, &stored.metadata.work_commit_oid])
            .unwrap();

        let orphans = anchor.scan_orphan_snapshot_refs(&known).unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].refname, orphan_ref);
        let err = anchor.run_guarded_gc(&known).unwrap_err();
        assert!(err.to_string().contains("orphan snapshot refs"));

        anchor_repo.run(&["update-ref", "-d", orphan_ref]).unwrap();
        anchor_repo
            .run(&["update-ref", "-d", &stored.metadata.work_ref()])
            .unwrap();
        let missing = anchor.missing_snapshot_refs(&known).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].refname, stored.metadata.work_ref());
        let err = anchor.run_guarded_gc(&known).unwrap_err();
        assert!(err.to_string().contains("missing snapshot refs"));
    }

    #[test]
    fn anchor_repo_rejects_unsafe_project_ids() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let err = AnchorSnapshotRepo::open(&home, "../bad").unwrap_err();

        assert!(err.to_string().contains("not safe for a repository path"));
    }
}
