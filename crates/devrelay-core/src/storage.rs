//! SQLite metadata storage and migrations.
//!
//! M1 stores local registry, workspace, session, snapshot, lease, and handoff
//! metadata in a per-project SQLite database. Migrations are monotonic and run
//! inside a transaction so a failed migration leaves the previous schema intact.

use crate::{DeviceIdentity, Result};
use rusqlite::{Connection, Transaction};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial_metadata",
        sql: r#"
CREATE TABLE projects (
    project_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    manifest_path TEXT,
    remote_fingerprint TEXT,
    root_fingerprint TEXT,
    created_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE workspaces (
    workspace_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    device_id TEXT,
    local_path TEXT NOT NULL,
    platform_profile TEXT NOT NULL,
    state TEXT NOT NULL,
    last_seen_head TEXT,
    last_checkpoint_id TEXT,
    created_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(local_path),
    FOREIGN KEY(project_id) REFERENCES projects(project_id)
);

CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    active_workspace_id TEXT,
    state TEXT NOT NULL,
    created_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY(project_id) REFERENCES projects(project_id),
    FOREIGN KEY(active_workspace_id) REFERENCES workspaces(workspace_id)
);

CREATE TABLE snapshots (
    snapshot_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    session_id TEXT,
    parent_snapshot_id TEXT,
    sequence_number INTEGER NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0,
    label TEXT,
    metadata_json TEXT NOT NULL,
    created_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY(project_id) REFERENCES projects(project_id),
    FOREIGN KEY(session_id) REFERENCES sessions(session_id),
    FOREIGN KEY(parent_snapshot_id) REFERENCES snapshots(snapshot_id)
);

CREATE TABLE leases (
    lease_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    holder_workspace_id TEXT,
    state TEXT NOT NULL,
    expires_at_unix_seconds INTEGER,
    created_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY(project_id) REFERENCES projects(project_id),
    FOREIGN KEY(holder_workspace_id) REFERENCES workspaces(workspace_id)
);

CREATE TABLE handoffs (
    handoff_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    source_workspace_id TEXT,
    target_workspace_id TEXT,
    state TEXT NOT NULL,
    created_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY(project_id) REFERENCES projects(project_id),
    FOREIGN KEY(source_workspace_id) REFERENCES workspaces(workspace_id),
    FOREIGN KEY(target_workspace_id) REFERENCES workspaces(workspace_id)
);

CREATE INDEX idx_projects_display_name ON projects(display_name);
CREATE INDEX idx_workspaces_project_path ON workspaces(project_id, local_path);
CREATE INDEX idx_snapshots_project_timeline
    ON snapshots(project_id, session_id, sequence_number, created_at_unix_seconds);
"#,
    },
    Migration {
        version: 2,
        name: "anchor_metadata_schema",
        sql: r#"
CREATE TABLE devices (
    device_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    platform_key TEXT NOT NULL,
    architecture TEXT NOT NULL,
    capabilities_json TEXT NOT NULL DEFAULT '{}',
    paired_at_unix_seconds INTEGER,
    last_seen_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch())
);

ALTER TABLE projects ADD COLUMN local_path TEXT;
ALTER TABLE projects ADD COLUMN remote_url_fingerprint TEXT;
ALTER TABLE projects ADD COLUMN root_commit_fingerprint TEXT;

ALTER TABLE leases ADD COLUMN session_id TEXT;
ALTER TABLE leases ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE leases ADD COLUMN holder_device_id TEXT;
ALTER TABLE leases ADD COLUMN latest_snapshot_id TEXT;
ALTER TABLE leases ADD COLUMN handoff_id TEXT;

ALTER TABLE handoffs ADD COLUMN expected_epoch INTEGER;
ALTER TABLE handoffs ADD COLUMN source_device_id TEXT;
ALTER TABLE handoffs ADD COLUMN target_device_id TEXT;
ALTER TABLE handoffs ADD COLUMN expires_at_unix_seconds INTEGER;

CREATE TABLE task_runs (
    task_run_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    session_id TEXT,
    state TEXT NOT NULL,
    command TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY(project_id) REFERENCES projects(project_id),
    FOREIGN KEY(session_id) REFERENCES sessions(session_id)
);

CREATE INDEX idx_workspaces_device_project
    ON workspaces(device_id, project_id);
CREATE INDEX idx_leases_project_state
    ON leases(project_id, state);
CREATE INDEX idx_leases_holder_device
    ON leases(project_id, holder_device_id, state);
CREATE INDEX idx_leases_session
    ON leases(project_id, session_id);
CREATE INDEX idx_snapshots_project_latest
    ON snapshots(project_id, sequence_number DESC);
CREATE INDEX idx_handoffs_project_state
    ON handoffs(project_id, state);
CREATE INDEX idx_handoffs_source_device_state
    ON handoffs(source_device_id, state);
CREATE INDEX idx_handoffs_target_device_state
    ON handoffs(target_device_id, state);
"#,
    },
];

pub struct MetadataDb {
    conn: Connection,
}

impl MetadataDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let mut db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn run_migrations(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch())
);
"#,
        )?;

        let applied = {
            let mut statement = tx.prepare("SELECT version FROM schema_migrations")?;
            let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
            rows.collect::<rusqlite::Result<BTreeSet<_>>>()?
        };

        for migration in MIGRATIONS {
            if applied.contains(&migration.version) {
                continue;
            }
            tx.execute_batch(migration.sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                (migration.version, migration.name),
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn transaction<T>(&mut self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let tx = self.conn.transaction()?;
        let result = f(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn upsert_device_identity(&self, device: &DeviceIdentity) -> Result<()> {
        self.conn.execute(
            r#"
INSERT INTO devices (
    device_id,
    display_name,
    platform_key,
    architecture,
    capabilities_json,
    paired_at_unix_seconds,
    last_seen_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(device_id) DO UPDATE SET
    display_name = excluded.display_name,
    platform_key = excluded.platform_key,
    architecture = excluded.architecture,
    capabilities_json = excluded.capabilities_json,
    paired_at_unix_seconds = excluded.paired_at_unix_seconds,
    last_seen_unix_seconds = excluded.last_seen_unix_seconds
"#,
            (
                device.device_id.as_str(),
                device.display_name.as_str(),
                device.platform_key.as_str(),
                device.architecture.as_str(),
                device.capabilities_json.as_str(),
                device.paired_at_unix_seconds.map(|value| value as i64),
                device.last_seen_unix_seconds as i64,
            ),
        )?;
        Ok(())
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceIdentity>> {
        let mut statement = self.conn.prepare(
            r#"
SELECT device_id,
       display_name,
       platform_key,
       architecture,
       capabilities_json,
       paired_at_unix_seconds,
       last_seen_unix_seconds
FROM devices
ORDER BY display_name ASC, device_id ASC
"#,
        )?;
        let rows = statement.query_map([], device_identity_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_device(&self, device_id: &str) -> Result<Option<DeviceIdentity>> {
        let mut statement = self.conn.prepare(
            r#"
SELECT device_id,
       display_name,
       platform_key,
       architecture,
       capabilities_json,
       paired_at_unix_seconds,
       last_seen_unix_seconds
FROM devices
WHERE device_id = ?1
"#,
        )?;
        let mut rows = statement.query([device_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(device_identity_from_row(row)?))
        } else {
            Ok(None)
        }
    }
}

fn device_identity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceIdentity> {
    let paired_at_unix_seconds = row
        .get::<_, Option<i64>>(5)?
        .map(|value| value.max(0) as u64);
    let last_seen_unix_seconds = row.get::<_, i64>(6)?.max(0) as u64;
    Ok(DeviceIdentity {
        device_id: row.get(0)?,
        display_name: row.get(1)?,
        platform_key: row.get(2)?,
        architecture: row.get(3)?,
        capabilities_json: row.get(4)?,
        paired_at_unix_seconds,
        last_seen_unix_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LocalConfig;

    fn table_exists(db: &MetadataDb, table: &str) -> bool {
        db.connection()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
    }

    fn index_exists(db: &MetadataDb, index: &str) -> bool {
        db.connection()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
                [index],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
    }

    fn column_exists(db: &MetadataDb, table: &str, column: &str) -> bool {
        let mut statement = db
            .connection()
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .iter()
            .any(|name| name == column)
    }

    fn foreign_key_exists(db: &MetadataDb, table: &str, from: &str, target: &str) -> bool {
        let mut statement = db
            .connection()
            .prepare(&format!("PRAGMA foreign_key_list({table})"))
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(2)?, row.get::<_, String>(3)?))
            })
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .iter()
            .any(|(target_table, from_column)| target_table == target && from_column == from)
    }

    #[test]
    fn migrates_empty_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();

        assert!(path.exists());
        for table in [
            "schema_migrations",
            "projects",
            "workspaces",
            "sessions",
            "snapshots",
            "leases",
            "handoffs",
            "devices",
            "task_runs",
        ] {
            assert!(table_exists(&db, table), "{table} should exist");
        }
        assert!(index_exists(&db, "idx_projects_display_name"));
        assert!(index_exists(&db, "idx_snapshots_project_timeline"));
        assert!(index_exists(&db, "idx_snapshots_project_latest"));
        assert!(index_exists(&db, "idx_leases_project_state"));
        assert!(index_exists(&db, "idx_leases_holder_device"));
        assert!(index_exists(&db, "idx_leases_session"));
        assert!(index_exists(&db, "idx_handoffs_project_state"));
        assert!(index_exists(&db, "idx_handoffs_source_device_state"));
        assert!(index_exists(&db, "idx_handoffs_target_device_state"));

        let journal_mode: String = db
            .connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    }

    #[test]
    fn schema_matches_anchor_metadata_contract() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();

        for column in [
            "device_id",
            "display_name",
            "platform_key",
            "architecture",
            "capabilities_json",
            "paired_at_unix_seconds",
            "last_seen_unix_seconds",
        ] {
            assert!(column_exists(&db, "devices", column), "{column}");
        }
        for column in [
            "project_id",
            "display_name",
            "local_path",
            "manifest_path",
            "remote_url_fingerprint",
            "root_commit_fingerprint",
        ] {
            assert!(column_exists(&db, "projects", column), "{column}");
        }
        assert!(column_exists(&db, "workspaces", "device_id"));
        assert!(column_exists(&db, "sessions", "session_id"));
        assert!(column_exists(&db, "snapshots", "sequence_number"));
        for column in [
            "epoch",
            "holder_device_id",
            "latest_snapshot_id",
            "handoff_id",
        ] {
            assert!(column_exists(&db, "leases", column), "{column}");
        }
        for column in [
            "expected_epoch",
            "source_device_id",
            "target_device_id",
            "expires_at_unix_seconds",
        ] {
            assert!(column_exists(&db, "handoffs", column), "{column}");
        }
        assert!(column_exists(&db, "task_runs", "task_run_id"));
        assert!(column_exists(&db, "task_runs", "metadata_json"));
    }

    #[test]
    fn metadata_tables_have_foreign_keys() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();

        assert!(foreign_key_exists(
            &db,
            "workspaces",
            "project_id",
            "projects"
        ));
        assert!(foreign_key_exists(
            &db,
            "sessions",
            "project_id",
            "projects"
        ));
        assert!(foreign_key_exists(
            &db,
            "sessions",
            "active_workspace_id",
            "workspaces"
        ));
        assert!(foreign_key_exists(
            &db,
            "snapshots",
            "project_id",
            "projects"
        ));
        assert!(foreign_key_exists(
            &db,
            "snapshots",
            "session_id",
            "sessions"
        ));
        assert!(foreign_key_exists(&db, "leases", "project_id", "projects"));
        assert!(foreign_key_exists(
            &db,
            "handoffs",
            "source_workspace_id",
            "workspaces"
        ));
        assert!(foreign_key_exists(
            &db,
            "task_runs",
            "project_id",
            "projects"
        ));
        assert!(foreign_key_exists(
            &db,
            "task_runs",
            "session_id",
            "sessions"
        ));
    }

    #[test]
    fn opens_database_in_wal_mode() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();

        let journal_mode: String = db
            .connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    }

    #[test]
    fn stores_device_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();
        let mut identity = LocalConfig::new_for_local_device().device_identity();
        identity.display_name = "Laptop".to_string();

        db.upsert_device_identity(&identity).unwrap();
        let devices = db.list_devices().unwrap();

        assert_eq!(devices, vec![identity.clone()]);
        assert_eq!(
            db.get_device(&identity.device_id).unwrap().as_ref(),
            Some(&identity)
        );

        let mut renamed = identity.clone();
        renamed.display_name = "Renamed Laptop".to_string();
        db.upsert_device_identity(&renamed).unwrap();

        let devices = db.list_devices().unwrap();
        assert_eq!(devices, vec![renamed]);
    }

    #[test]
    fn migrations_are_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let mut db = MetadataDb::open(&path).unwrap();

        db.run_migrations().unwrap();
        db.run_migrations().unwrap();

        let count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn transaction_helper_commits_on_success() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let mut db = MetadataDb::open(&path).unwrap();

        db.transaction(|tx| {
            tx.execute(
                "INSERT INTO projects (project_id, display_name) VALUES (?1, ?2)",
                ("project123", "Demo"),
            )?;
            Ok(())
        })
        .unwrap();

        let count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
