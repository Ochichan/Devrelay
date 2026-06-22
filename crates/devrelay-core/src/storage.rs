//! SQLite metadata storage and migrations.
//!
//! M1 stores local registry, workspace, session, snapshot, lease, and handoff
//! metadata in a per-project SQLite database. Migrations are monotonic and run
//! inside a transaction so a failed migration leaves the previous schema intact.

use crate::Result;
use rusqlite::{Connection, Transaction};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
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
}];

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ] {
            assert!(table_exists(&db, table), "{table} should exist");
        }
        assert!(index_exists(&db, "idx_projects_display_name"));
        assert!(index_exists(&db, "idx_snapshots_project_timeline"));

        let journal_mode: String = db
            .connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
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
        assert_eq!(count, 1);
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
