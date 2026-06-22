//! Per-project local snapshot store.
//!
//! M1 keeps durable snapshot objects in a project-local bare repository under
//! `DEVRELAY_HOME` and stores queryable snapshot metadata in SQLite. The source
//! worktree only needs synthetic refs long enough for this store to import the
//! objects.

use crate::{
    DevRelayError, DevRelayHome, GitRepo, Manifest, MetadataDb, Result, SnapshotMetadata,
    create_snapshot, write_snapshot_file,
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

pub struct SnapshotStore {
    project_id: String,
    snapshot_repo_path: PathBuf,
    db: MetadataDb,
}

impl SnapshotStore {
    pub fn open(home: &DevRelayHome, project_id: &str) -> Result<Self> {
        home.create_project_dirs(project_id)?;
        let snapshot_repo_path = home.snapshot_bare_repo_path(project_id);
        ensure_bare_repo(&snapshot_repo_path)?;
        let db = MetadataDb::open(home.metadata_db_path(project_id))?;
        Ok(Self {
            project_id: project_id.to_string(),
            snapshot_repo_path,
            db,
        })
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
        if metadata.parent_snapshot_id.is_none() {
            metadata.parent_snapshot_id = self.latest_snapshot_id()?;
        }

        let stored = self.insert_snapshot(metadata, pinned, label)?;
        remove_source_snapshot_refs(source, &stored.metadata)?;
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

    fn insert_snapshot(
        &mut self,
        metadata: SnapshotMetadata,
        pinned: bool,
        label: Option<String>,
    ) -> Result<StoredSnapshot> {
        let metadata_json = serde_json::to_string(&metadata)?;
        self.db.transaction(|tx| {
            tx.execute(
                "INSERT OR IGNORE INTO projects (project_id, display_name) VALUES (?1, ?2)",
                (metadata.project_id.as_str(), metadata.project_name.as_str()),
            )?;
            tx.execute(
                "UPDATE projects SET display_name = ?1 WHERE project_id = ?2",
                (metadata.project_name.as_str(), metadata.project_id.as_str()),
            )?;
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
        Ok(self
            .db
            .connection()
            .query_row(
                r#"
SELECT snapshot_id
FROM snapshots
WHERE project_id = ?1
ORDER BY sequence_number DESC
LIMIT 1
"#,
                [self.project_id.as_str()],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn snapshot_repo(&self) -> GitRepo {
        GitRepo::new(&self.snapshot_repo_path)
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

fn remove_source_snapshot_refs(source: &GitRepo, metadata: &SnapshotMetadata) -> Result<()> {
    source.run(&["update-ref", "-d", &metadata.index_ref()])?;
    source.run(&["update-ref", "-d", &metadata.work_ref()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
