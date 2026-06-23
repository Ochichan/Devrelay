//! Per-project local snapshot store.
//!
//! M1 keeps durable snapshot objects in a project-local bare repository under
//! `DEVRELAY_HOME` and stores queryable snapshot metadata in SQLite. The source
//! worktree only needs synthetic refs long enough for this store to import the
//! objects.

use crate::snapshot_schema::calculate_state_hash;
use crate::{
    DevRelayError, DevRelayHome, GitRepo, Manifest, MetadataDb, PruningDecisionAction, PruningPlan,
    Result, SUBMODULE_CHILD_SNAPSHOT_RELATIONSHIP, SnapshotChildSnapshot, SnapshotMetadata,
    StoredSession, SubmoduleStatus, create_snapshot, dirty_submodule_child_manifest,
    inspect_submodules, write_snapshot_file,
};
use rusqlite::{OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSnapshot {
    pub snapshot_id: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub sequence_number: i64,
    pub pinned: bool,
    pub label: Option<String>,
    pub metadata: SnapshotMetadata,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotPruneResult {
    pub deleted_snapshot_ids: Vec<String>,
    pub reclaimed_estimated_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum SnapshotCheckpointResult {
    Created {
        snapshot: Box<StoredSnapshot>,
    },
    Unchanged {
        snapshot_id: String,
        state_hash: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotCheckpointWithChildren {
    pub parent: StoredSnapshot,
    pub children: Vec<StoredSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotStoreFaultPoint {
    AfterSnapshotObjectImport,
    DuringMetadataTransaction,
    AfterSourceIndexRefDelete,
}

impl SnapshotStoreFaultPoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::AfterSnapshotObjectImport => "after-snapshot-object-import",
            Self::DuringMetadataTransaction => "during-metadata-transaction",
            Self::AfterSourceIndexRefDelete => "after-source-index-ref-delete",
        }
    }
}

pub struct SnapshotStore {
    home: DevRelayHome,
    project_id: String,
    snapshot_repo_path: PathBuf,
    db: MetadataDb,
    fault: Option<SnapshotStoreFaultPoint>,
}

impl SnapshotStore {
    pub fn open(home: &DevRelayHome, project_id: &str) -> Result<Self> {
        home.create_project_dirs(project_id)?;
        let snapshot_repo_path = home.snapshot_bare_repo_path(project_id);
        ensure_bare_repo(&snapshot_repo_path)?;
        let db = MetadataDb::open(home.metadata_db_path(project_id))?;
        Ok(Self {
            home: home.clone(),
            project_id: project_id.to_string(),
            snapshot_repo_path,
            db,
            fault: None,
        })
    }

    pub fn with_fault_injection(mut self, fault: SnapshotStoreFaultPoint) -> Self {
        self.fault = Some(fault);
        self
    }

    pub fn set_fault_injection(&mut self, fault: Option<SnapshotStoreFaultPoint>) {
        self.fault = fault;
    }

    pub fn snapshot_repo_path(&self) -> &Path {
        &self.snapshot_repo_path
    }

    pub fn checkpoint(
        &mut self,
        source: &GitRepo,
        manifest: &Manifest,
        pinned: bool,
        label: Option<String>,
    ) -> Result<StoredSnapshot> {
        let metadata = create_snapshot(source, manifest)?;
        self.store_snapshot(source, metadata, pinned, label)
    }

    pub fn checkpoint_with_dirty_submodules(
        &mut self,
        source: &GitRepo,
        manifest: &Manifest,
        pinned: bool,
        label: Option<String>,
    ) -> Result<SnapshotCheckpointWithChildren> {
        let (child_links, children) =
            self.checkpoint_dirty_submodule_children(source, manifest, pinned)?;
        let mut metadata = create_snapshot(source, manifest)?;
        metadata.child_snapshots = child_links;
        metadata.child_snapshots.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.project_id.cmp(&right.project_id))
                .then(left.session_id.cmp(&right.session_id))
                .then(left.snapshot_id.cmp(&right.snapshot_id))
        });
        metadata.state_hash = calculate_state_hash(&metadata);
        metadata.validate()?;

        let parent = self.store_snapshot(source, metadata, pinned, label)?;
        Ok(SnapshotCheckpointWithChildren { parent, children })
    }

    pub fn checkpoint_if_changed(
        &mut self,
        source: &GitRepo,
        manifest: &Manifest,
        pinned: bool,
        label: Option<String>,
    ) -> Result<SnapshotCheckpointResult> {
        let metadata = create_snapshot(source, manifest)?;
        if let Some(latest) = self.latest_snapshot()?
            && latest.metadata.state_hash == metadata.state_hash
        {
            let state_hash = metadata.state_hash.clone();
            remove_source_snapshot_refs(source, &metadata, None)?;
            return Ok(SnapshotCheckpointResult::Unchanged {
                snapshot_id: latest.snapshot_id,
                state_hash,
            });
        }

        self.store_snapshot(source, metadata, pinned, label)
            .map(|snapshot| SnapshotCheckpointResult::Created {
                snapshot: Box::new(snapshot),
            })
    }

    pub fn store_snapshot(
        &mut self,
        source: &GitRepo,
        mut metadata: SnapshotMetadata,
        pinned: bool,
        label: Option<String>,
    ) -> Result<StoredSnapshot> {
        if metadata.project_id != self.project_id {
            return Err(DevRelayError::Config(format!(
                "snapshot project_id {} does not match store project_id {}",
                metadata.project_id, self.project_id
            )));
        }
        metadata.validate()?;
        self.import_snapshot_refs(source, &metadata)?;
        self.inject_fault(SnapshotStoreFaultPoint::AfterSnapshotObjectImport)?;
        if metadata.parent_snapshot_id.is_none() {
            metadata.parent_snapshot_id = self.latest_snapshot_id()?;
        }

        let stored = self.insert_snapshot(metadata, pinned, label)?;
        remove_source_snapshot_refs(source, &stored.metadata, self.fault)?;
        Ok(stored)
    }

    pub fn import_snapshot_refs(
        &self,
        source: &GitRepo,
        metadata: &SnapshotMetadata,
    ) -> Result<()> {
        metadata.validate()?;
        let source_path = source.path().as_os_str().to_os_string();
        self.snapshot_repo().run_with_env(
            [
                OsString::from("fetch"),
                source_path,
                OsString::from(format!("{}:{}", metadata.index_ref(), metadata.index_ref())),
                OsString::from(format!("{}:{}", metadata.work_ref(), metadata.work_ref())),
            ],
            &[],
        )?;
        Ok(())
    }

    pub fn export_snapshot_refs(&self, target: &GitRepo, snapshot_id: &str) -> Result<()> {
        let stored = self.get_snapshot(snapshot_id)?;
        let source_path = self.snapshot_repo_path.as_os_str().to_os_string();
        target.run_with_env(
            [
                OsString::from("fetch"),
                source_path,
                OsString::from(format!(
                    "{}:{}",
                    stored.metadata.index_ref(),
                    stored.metadata.index_ref()
                )),
                OsString::from(format!(
                    "{}:{}",
                    stored.metadata.work_ref(),
                    stored.metadata.work_ref()
                )),
            ],
            &[],
        )?;
        Ok(())
    }

    pub fn export_snapshot_json(
        &self,
        snapshot_id: &str,
        path: impl AsRef<Path>,
    ) -> Result<StoredSnapshot> {
        let stored = self.get_snapshot(snapshot_id)?;
        write_snapshot_file(path, &stored.metadata)?;
        Ok(stored)
    }

    pub fn list_snapshots(&self) -> Result<Vec<StoredSnapshot>> {
        let mut statement = self.db.connection().prepare(
            r#"
SELECT snapshot_id,
       project_id,
       session_id,
       parent_snapshot_id,
       sequence_number,
       pinned,
       label,
       metadata_json,
       created_at_unix_seconds
FROM snapshots
WHERE project_id = ?1
ORDER BY sequence_number ASC
"#,
        )?;
        let rows = statement.query_map([self.project_id.as_str()], stored_snapshot_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_snapshot(&self, snapshot_id: &str) -> Result<StoredSnapshot> {
        self.db
            .connection()
            .query_row(
                r#"
SELECT snapshot_id,
       project_id,
       session_id,
       parent_snapshot_id,
       sequence_number,
       pinned,
       label,
       metadata_json,
       created_at_unix_seconds
FROM snapshots
WHERE project_id = ?1 AND snapshot_id = ?2
"#,
                (self.project_id.as_str(), snapshot_id),
                stored_snapshot_from_row,
            )
            .optional()?
            .ok_or_else(|| DevRelayError::Config(format!("unknown snapshot {snapshot_id}")))
    }

    pub fn latest_snapshot(&self) -> Result<Option<StoredSnapshot>> {
        Ok(self
            .db
            .connection()
            .query_row(
                r#"
SELECT snapshot_id,
       project_id,
       session_id,
       parent_snapshot_id,
       sequence_number,
       pinned,
       label,
       metadata_json,
       created_at_unix_seconds
FROM snapshots
WHERE project_id = ?1
ORDER BY sequence_number DESC
LIMIT 1
"#,
                [self.project_id.as_str()],
                stored_snapshot_from_row,
            )
            .optional()?)
    }

    pub fn prune_snapshots(&mut self, plan: &PruningPlan) -> Result<SnapshotPruneResult> {
        let latest_snapshot_id = self.latest_snapshot_id()?;
        let mut deleted_snapshot_ids = Vec::new();
        let mut reclaimed_estimated_bytes = 0_u64;

        for decision in &plan.decisions {
            let PruningDecisionAction::Delete { .. } = decision.action else {
                continue;
            };
            let stored = self.get_snapshot(&decision.snapshot_id)?;
            if stored.pinned {
                return Err(DevRelayError::Config(format!(
                    "refusing to prune pinned snapshot {}",
                    stored.snapshot_id
                )));
            }
            if latest_snapshot_id.as_deref() == Some(stored.snapshot_id.as_str()) {
                return Err(DevRelayError::Config(format!(
                    "refusing to prune latest snapshot {}",
                    stored.snapshot_id
                )));
            }
            if self.snapshot_has_children(&stored.snapshot_id)? {
                return Err(DevRelayError::Config(format!(
                    "refusing to prune snapshot {} while child snapshots still reference it",
                    stored.snapshot_id
                )));
            }

            self.delete_snapshot_refs(&stored.metadata)?;
            let deleted = self.db.connection().execute(
                "DELETE FROM snapshots WHERE project_id = ?1 AND snapshot_id = ?2 AND pinned = 0",
                (self.project_id.as_str(), stored.snapshot_id.as_str()),
            )?;
            if deleted == 0 {
                return Err(DevRelayError::Config(format!(
                    "snapshot {} metadata was not pruned",
                    stored.snapshot_id
                )));
            }
            deleted_snapshot_ids.push(stored.snapshot_id);
            reclaimed_estimated_bytes =
                reclaimed_estimated_bytes.saturating_add(decision.estimated_size_bytes);
        }

        Ok(SnapshotPruneResult {
            deleted_snapshot_ids,
            reclaimed_estimated_bytes,
        })
    }

    fn insert_snapshot(
        &mut self,
        metadata: SnapshotMetadata,
        pinned: bool,
        label: Option<String>,
    ) -> Result<StoredSnapshot> {
        let metadata_json = serde_json::to_string(&metadata)?;
        let fault = self.fault;
        self.db.transaction(|tx| {
            tx.execute(
                "INSERT OR IGNORE INTO projects (project_id, display_name) VALUES (?1, ?2)",
                (metadata.project_id.as_str(), metadata.project_name.as_str()),
            )?;
            tx.execute(
                "UPDATE projects SET display_name = ?1 WHERE project_id = ?2",
                (metadata.project_name.as_str(), metadata.project_id.as_str()),
            )?;
            inject_fault(fault, SnapshotStoreFaultPoint::DuringMetadataTransaction)?;
            if let Some(session_id) = metadata.session_id.as_deref() {
                tx.execute(
                    r#"
INSERT OR IGNORE INTO sessions (session_id, project_id, state)
VALUES (?1, ?2, ?3)
"#,
                    (session_id, metadata.project_id.as_str(), "fork"),
                )?;
            }
            let sequence_number: i64 = tx.query_row(
                "SELECT COALESCE(MAX(sequence_number), 0) + 1 FROM snapshots WHERE project_id = ?1",
                [metadata.project_id.as_str()],
                |row| row.get(0),
            )?;
            tx.execute(
                r#"
INSERT INTO snapshots (
    snapshot_id,
    project_id,
    session_id,
    parent_snapshot_id,
    sequence_number,
    pinned,
    label,
    metadata_json,
    created_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
"#,
                (
                    metadata.snapshot_id.as_str(),
                    metadata.project_id.as_str(),
                    metadata.session_id.as_deref(),
                    metadata.parent_snapshot_id.as_deref(),
                    sequence_number,
                    pinned,
                    label.as_deref(),
                    metadata_json.as_str(),
                    metadata.created_at_unix_seconds as i64,
                ),
            )?;
            Ok(StoredSnapshot {
                snapshot_id: metadata.snapshot_id.clone(),
                project_id: metadata.project_id.clone(),
                session_id: metadata.session_id.clone(),
                parent_snapshot_id: metadata.parent_snapshot_id.clone(),
                sequence_number,
                pinned,
                label,
                created_at_unix_seconds: metadata.created_at_unix_seconds,
                metadata,
            })
        })
    }

    fn latest_snapshot_id(&self) -> Result<Option<String>> {
        Ok(self.latest_snapshot()?.map(|snapshot| snapshot.snapshot_id))
    }

    fn snapshot_repo(&self) -> GitRepo {
        GitRepo::new(&self.snapshot_repo_path)
    }

    fn checkpoint_dirty_submodule_children(
        &self,
        source: &GitRepo,
        manifest: &Manifest,
        pinned: bool,
    ) -> Result<(Vec<SnapshotChildSnapshot>, Vec<StoredSnapshot>)> {
        let report = inspect_submodules(source)?;
        let dirty_submodules = report
            .submodules
            .into_iter()
            .filter(|state| state.status == SubmoduleStatus::Dirty)
            .collect::<Vec<_>>();
        let mut child_links = Vec::with_capacity(dirty_submodules.len());
        let mut children = Vec::with_capacity(dirty_submodules.len());

        for submodule in dirty_submodules {
            let child_manifest = dirty_submodule_child_manifest(manifest, &submodule.path);
            let child_repo = GitRepo::new(source.path().join(PathBuf::from(&submodule.path)));
            let mut child_store = SnapshotStore::open(&self.home, &child_manifest.project_id)?;
            let child_session = child_store.db.ensure_default_session(
                &child_manifest.project_id,
                &child_manifest.name,
                None,
            )?;
            let child_stored = child_store.checkpoint_child_submodule_snapshot(
                &child_repo,
                &child_manifest,
                &child_session,
                &submodule.path,
                pinned,
            )?;

            child_links.push(SnapshotChildSnapshot {
                relationship: SUBMODULE_CHILD_SNAPSHOT_RELATIONSHIP.to_string(),
                path: submodule.path,
                project_id: child_stored.project_id.clone(),
                session_id: child_session.session_id,
                snapshot_id: child_stored.snapshot_id.clone(),
                state_hash: child_stored.metadata.state_hash.clone(),
            });
            children.push(child_stored);
        }

        Ok((child_links, children))
    }

    fn checkpoint_child_submodule_snapshot(
        &mut self,
        source: &GitRepo,
        manifest: &Manifest,
        session: &StoredSession,
        submodule_path: &str,
        pinned: bool,
    ) -> Result<StoredSnapshot> {
        let mut metadata = create_snapshot(source, manifest)?;
        metadata.session_id = Some(session.session_id.clone());
        metadata.state_hash = calculate_state_hash(&metadata);
        metadata.validate()?;
        self.store_snapshot(
            source,
            metadata,
            pinned,
            Some(format!("dirty submodule {submodule_path}")),
        )
    }

    fn delete_snapshot_refs(&self, metadata: &SnapshotMetadata) -> Result<()> {
        let repo = self.snapshot_repo();
        repo.run(&["update-ref", "-d", &metadata.index_ref()])?;
        repo.run(&["update-ref", "-d", &metadata.work_ref()])?;
        Ok(())
    }

    fn snapshot_has_children(&self, snapshot_id: &str) -> Result<bool> {
        self.db
            .connection()
            .query_row(
                r#"
SELECT EXISTS(
    SELECT 1
    FROM snapshots
    WHERE project_id = ?1 AND parent_snapshot_id = ?2
)
"#,
                (self.project_id.as_str(), snapshot_id),
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    fn inject_fault(&self, fault: SnapshotStoreFaultPoint) -> Result<()> {
        inject_fault(self.fault, fault)
    }
}

fn stored_snapshot_from_row(row: &Row<'_>) -> rusqlite::Result<StoredSnapshot> {
    let metadata_json: String = row.get(7)?;
    let metadata: SnapshotMetadata = serde_json::from_str(&metadata_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(err))
    })?;
    let created_at_unix_seconds: i64 = row.get(8)?;
    Ok(StoredSnapshot {
        snapshot_id: row.get(0)?,
        project_id: row.get(1)?,
        session_id: row.get(2)?,
        parent_snapshot_id: row.get(3)?,
        sequence_number: row.get(4)?,
        pinned: row.get(5)?,
        label: row.get(6)?,
        metadata,
        created_at_unix_seconds: created_at_unix_seconds as u64,
    })
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

fn remove_source_snapshot_refs(
    source: &GitRepo,
    metadata: &SnapshotMetadata,
    fault: Option<SnapshotStoreFaultPoint>,
) -> Result<()> {
    source.run(&["update-ref", "-d", &metadata.index_ref()])?;
    inject_fault(fault, SnapshotStoreFaultPoint::AfterSourceIndexRefDelete)?;
    source.run(&["update-ref", "-d", &metadata.work_ref()])?;
    Ok(())
}

fn inject_fault(
    configured: Option<SnapshotStoreFaultPoint>,
    fault: SnapshotStoreFaultPoint,
) -> Result<()> {
    if configured == Some(fault) {
        return Err(injected_fault(fault));
    }
    Ok(())
}

fn injected_fault(fault: SnapshotStoreFaultPoint) -> DevRelayError {
    DevRelayError::Config(format!("injected fault at {}", fault.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PruningDecision, PruningReason, PruningScope, RetentionPolicy, SnapshotRetentionEntry,
        plan_snapshot_pruning,
    };
    use std::fs;

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"
schema = 1
project_id = "store-project"
name = "Store Project"

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

    fn delete_plan(snapshot: &StoredSnapshot) -> PruningPlan {
        PruningPlan {
            scope: PruningScope::DeviceCache,
            current_usage_bytes: snapshot.metadata.created_at_unix_seconds,
            free_disk_bytes: 0,
            target_reclaim_bytes: 1,
            planned_reclaim_bytes: 1,
            decisions: vec![PruningDecision {
                snapshot_id: snapshot.snapshot_id.clone(),
                estimated_size_bytes: 1,
                action: PruningDecisionAction::Delete {
                    reason: PruningReason::QuotaPressure,
                },
            }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn stores_snapshot_refs_and_metadata_outside_source_repo() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let manifest = manifest();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();
        let first = store
            .checkpoint(&source, &manifest, true, Some("first".to_string()))
            .unwrap();

        assert!(store.snapshot_repo_path().join("HEAD").exists());
        assert_eq!(first.sequence_number, 1);
        assert!(first.pinned);
        assert_eq!(first.label.as_deref(), Some("first"));
        assert_eq!(first.parent_snapshot_id, None);
        assert!(
            source
                .run(&["rev-parse", "--verify", &first.metadata.index_ref()])
                .is_err()
        );
        assert!(
            store
                .snapshot_repo()
                .run(&["rev-parse", "--verify", &first.metadata.index_ref()])
                .is_ok()
        );

        fs::write(source_path.join("tracked.txt"), "changed again\n").unwrap();
        let second = store.checkpoint(&source, &manifest, false, None).unwrap();
        assert_eq!(second.sequence_number, 2);
        assert_eq!(
            second.parent_snapshot_id.as_deref(),
            Some(first.snapshot_id.as_str())
        );

        let listed = store.list_snapshots().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].snapshot_id, first.snapshot_id);
        assert_eq!(listed[1].snapshot_id, second.snapshot_id);

        store
            .export_snapshot_refs(&source, &second.snapshot_id)
            .unwrap();
        assert!(
            source
                .run(&["rev-parse", "--verify", &second.metadata.work_ref()])
                .is_ok()
        );

        let export_path = temp.path().join("snapshot.json");
        let exported = store
            .export_snapshot_json(&second.snapshot_id, &export_path)
            .unwrap();
        assert_eq!(exported.snapshot_id, second.snapshot_id);
        assert!(export_path.exists());
    }

    #[test]
    fn checkpoint_with_dirty_submodules_creates_child_project_session_and_topology() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let child_source_path = temp.path().join("child-source");
        let parent_path = temp.path().join("parent");
        let child_source = init_repo(&child_source_path);
        fs::write(child_source_path.join("tracked.txt"), "child base\n").unwrap();
        child_source.run(&["add", "tracked.txt"]).unwrap();
        child_source.run(&["commit", "-m", "child base"]).unwrap();
        let parent = init_repo(&parent_path);
        parent
            .run_with_env(
                [
                    OsString::from("submodule"),
                    OsString::from("add"),
                    child_source_path.as_os_str().to_os_string(),
                    OsString::from("deps/child"),
                ],
                &[("GIT_ALLOW_PROTOCOL", std::ffi::OsStr::new("file"))],
            )
            .unwrap();
        parent.run(&["commit", "-m", "add submodule"]).unwrap();
        fs::write(parent_path.join("deps/child/tracked.txt"), "child dirty\n").unwrap();
        fs::write(parent_path.join("deps/child/notes.md"), "child note\n").unwrap();

        let manifest = manifest();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();
        let result = store
            .checkpoint_with_dirty_submodules(&parent, &manifest, false, Some("parent".to_string()))
            .unwrap();

        assert_eq!(result.children.len(), 1);
        let link = &result.parent.metadata.child_snapshots[0];
        let child = &result.children[0];
        assert_eq!(link.relationship, SUBMODULE_CHILD_SNAPSHOT_RELATIONSHIP);
        assert_eq!(link.path, "deps/child");
        assert_eq!(link.project_id, child.project_id);
        assert_eq!(link.session_id, child.session_id.as_deref().unwrap());
        assert_eq!(link.snapshot_id, child.snapshot_id);
        assert_eq!(link.state_hash, child.metadata.state_hash);
        assert_eq!(
            result.parent.metadata.state_hash,
            calculate_state_hash(&result.parent.metadata)
        );

        let stored_parent = store.get_snapshot(&result.parent.snapshot_id).unwrap();
        assert_eq!(stored_parent.metadata.child_snapshots, vec![link.clone()]);

        let child_store = SnapshotStore::open(&home, &link.project_id).unwrap();
        let child_snapshots = child_store.list_snapshots().unwrap();
        assert_eq!(child_snapshots.len(), 1);
        assert_eq!(child_snapshots[0].snapshot_id, child.snapshot_id);
        let child_sessions = child_store
            .db
            .list_sessions(Some(&link.project_id))
            .unwrap();
        assert_eq!(child_sessions.len(), 1);
        assert_eq!(child_sessions[0].session_id, link.session_id);

        let tracked = child_store
            .snapshot_repo()
            .run(&[
                "show",
                &format!("{}:tracked.txt", child.metadata.work_commit_oid),
            ])
            .unwrap();
        let note = child_store
            .snapshot_repo()
            .run(&[
                "show",
                &format!("{}:notes.md", child.metadata.work_commit_oid),
            ])
            .unwrap();
        assert_eq!(tracked, "child dirty");
        assert_eq!(note, "child note");
    }

    #[test]
    fn checkpoint_if_changed_skips_duplicate_semantic_state() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let manifest = manifest();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let first = store
            .checkpoint_if_changed(&source, &manifest, false, Some("first".to_string()))
            .unwrap();
        let first = match first {
            SnapshotCheckpointResult::Created { snapshot } => *snapshot,
            SnapshotCheckpointResult::Unchanged { .. } => panic!("first checkpoint must store"),
        };

        let duplicate = store
            .checkpoint_if_changed(&source, &manifest, false, Some("duplicate".to_string()))
            .unwrap();

        assert_eq!(
            duplicate,
            SnapshotCheckpointResult::Unchanged {
                snapshot_id: first.snapshot_id.clone(),
                state_hash: first.metadata.state_hash.clone(),
            }
        );
        assert_eq!(store.list_snapshots().unwrap().len(), 1);
        assert!(
            source
                .run(&["rev-parse", "--verify", &first.metadata.index_ref()])
                .is_err()
        );
        assert!(
            source
                .run(&["rev-parse", "--verify", &first.metadata.work_ref()])
                .is_err()
        );
    }

    #[test]
    fn fault_after_snapshot_object_import_does_not_publish_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let manifest = manifest();
        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let mut store = SnapshotStore::open(&home, &manifest.project_id)
            .unwrap()
            .with_fault_injection(SnapshotStoreFaultPoint::AfterSnapshotObjectImport);

        let err = store
            .checkpoint(&source, &manifest, true, Some("fault".to_string()))
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("injected fault at after-snapshot-object-import")
        );
        assert!(store.list_snapshots().unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(source_path.join("tracked.txt")).unwrap(),
            "changed\n"
        );
        let imported_refs = store
            .snapshot_repo()
            .run(&[
                "for-each-ref",
                "--format=%(refname)",
                "refs/devrelay/snapshots",
            ])
            .unwrap();
        assert!(
            imported_refs.contains("refs/devrelay/snapshots/"),
            "expected imported object refs to remain recoverable"
        );
    }

    #[test]
    fn fault_during_metadata_transaction_rolls_back_snapshot_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let manifest = manifest();
        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let mut store = SnapshotStore::open(&home, &manifest.project_id)
            .unwrap()
            .with_fault_injection(SnapshotStoreFaultPoint::DuringMetadataTransaction);

        let err = store
            .checkpoint(&source, &manifest, true, Some("fault".to_string()))
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("injected fault at during-metadata-transaction")
        );
        assert!(store.list_snapshots().unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(source_path.join("tracked.txt")).unwrap(),
            "changed\n"
        );
        let imported_refs = store
            .snapshot_repo()
            .run(&[
                "for-each-ref",
                "--format=%(refname)",
                "refs/devrelay/snapshots",
            ])
            .unwrap();
        assert!(
            imported_refs.contains("refs/devrelay/snapshots/"),
            "object refs should remain available for inspection after rollback"
        );
    }

    #[test]
    fn fault_during_source_ref_cleanup_preserves_snapshot_and_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let manifest = manifest();
        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let mut store = SnapshotStore::open(&home, &manifest.project_id)
            .unwrap()
            .with_fault_injection(SnapshotStoreFaultPoint::AfterSourceIndexRefDelete);

        let err = store
            .checkpoint(&source, &manifest, true, Some("fault".to_string()))
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("injected fault at after-source-index-ref-delete")
        );
        let snapshots = store.list_snapshots().unwrap();
        assert_eq!(snapshots.len(), 1);
        let snapshot = &snapshots[0];
        assert_eq!(
            fs::read_to_string(source_path.join("tracked.txt")).unwrap(),
            "changed\n"
        );
        assert!(
            store
                .snapshot_repo()
                .run(&["rev-parse", "--verify", &snapshot.metadata.index_ref()])
                .is_ok()
        );
        assert!(
            store
                .snapshot_repo()
                .run(&["rev-parse", "--verify", &snapshot.metadata.work_ref()])
                .is_ok()
        );
        assert!(
            source
                .run(&["rev-parse", "--verify", &snapshot.metadata.index_ref()])
                .is_err()
        );
        assert!(
            source
                .run(&["rev-parse", "--verify", &snapshot.metadata.work_ref()])
                .is_ok()
        );
    }

    #[test]
    fn pruning_executor_deletes_planned_snapshot_refs_and_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let manifest = manifest();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();

        fs::write(source_path.join("tracked.txt"), "first\n").unwrap();
        let first = store.checkpoint(&source, &manifest, false, None).unwrap();
        fs::write(source_path.join("tracked.txt"), "second\n").unwrap();
        let mut leaf_metadata = create_snapshot(&source, &manifest).unwrap();
        leaf_metadata.parent_snapshot_id = Some(first.snapshot_id.clone());
        let leaf = store
            .store_snapshot(&source, leaf_metadata, false, Some("leaf".to_string()))
            .unwrap();
        fs::write(source_path.join("tracked.txt"), "third\n").unwrap();
        let mut latest_metadata = create_snapshot(&source, &manifest).unwrap();
        latest_metadata.parent_snapshot_id = Some(first.snapshot_id.clone());
        let latest = store
            .store_snapshot(&source, latest_metadata, true, Some("latest".to_string()))
            .unwrap();

        let policy = RetentionPolicy {
            hot_snapshot_count: 1,
            hourly_thinning_hours: 0,
            daily_thinning_days: 0,
            ..RetentionPolicy::default()
        };
        let snapshots = vec![
            SnapshotRetentionEntry::from_stored(&first, 10),
            SnapshotRetentionEntry::from_stored(&leaf, 20),
            SnapshotRetentionEntry::from_stored(&latest, 30),
        ];
        let plan = plan_snapshot_pruning(crate::PruningPlanInput {
            snapshots: &snapshots,
            policy,
            scope: PruningScope::DeviceCache,
            canonical_latest_snapshot_id: Some(&latest.snapshot_id),
            handoff_protections: &[],
            current_usage_bytes: 60,
            free_disk_bytes: 1024 * 1024 * 1024,
            now_unix_seconds: latest.created_at_unix_seconds.saturating_add(1),
        });

        let result = store.prune_snapshots(&plan).unwrap();

        assert_eq!(result.deleted_snapshot_ids, vec![leaf.snapshot_id.clone()]);
        assert_eq!(result.reclaimed_estimated_bytes, 20);
        assert!(store.get_snapshot(&leaf.snapshot_id).is_err());
        assert!(
            store
                .snapshot_repo()
                .run(&["rev-parse", "--verify", &leaf.metadata.index_ref()])
                .is_err()
        );
        assert!(
            store
                .snapshot_repo()
                .run(&["rev-parse", "--verify", &leaf.metadata.work_ref()])
                .is_err()
        );
        let remaining = store
            .list_snapshots()
            .unwrap()
            .into_iter()
            .map(|snapshot| snapshot.snapshot_id)
            .collect::<Vec<_>>();
        assert_eq!(remaining, vec![first.snapshot_id, latest.snapshot_id]);
    }

    #[test]
    fn pruning_executor_refuses_pinned_and_latest_snapshots() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let manifest = manifest();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();

        fs::write(source_path.join("tracked.txt"), "pinned\n").unwrap();
        let pinned = store
            .checkpoint(&source, &manifest, true, Some("pinned".to_string()))
            .unwrap();
        fs::write(source_path.join("tracked.txt"), "latest\n").unwrap();
        let latest = store.checkpoint(&source, &manifest, false, None).unwrap();

        let pinned_err = store.prune_snapshots(&delete_plan(&pinned)).unwrap_err();
        assert!(pinned_err.to_string().contains("pinned snapshot"));

        let latest_err = store.prune_snapshots(&delete_plan(&latest)).unwrap_err();
        assert!(latest_err.to_string().contains("latest snapshot"));

        let remaining = store
            .list_snapshots()
            .unwrap()
            .into_iter()
            .map(|snapshot| snapshot.snapshot_id)
            .collect::<Vec<_>>();
        assert_eq!(remaining, vec![pinned.snapshot_id, latest.snapshot_id]);
    }
}
