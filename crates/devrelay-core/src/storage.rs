//! SQLite metadata storage and migrations.
//!
//! M1 stores local registry, workspace, session, snapshot, lease, and handoff
//! metadata in a per-project SQLite database. Migrations are monotonic and run
//! inside a transaction so a failed migration leaves the previous schema intact.

use crate::{
    DeviceIdentity, HandoffRecord, HandoffState, LeaseRecord, LeaseState, Result, SessionState,
    SnapshotMetadata, StoredSession, StoredSnapshot, generate_handoff_id, generate_session_id,
    unix_now_seconds,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction};
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
    Migration {
        version: 3,
        name: "session_model",
        sql: r#"
ALTER TABLE sessions ADD COLUMN name TEXT;
ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;
ALTER TABLE sessions ADD COLUMN archived_at_unix_seconds INTEGER;

CREATE INDEX idx_sessions_project_state
    ON sessions(project_id, state);
CREATE INDEX idx_sessions_parent
    ON sessions(parent_session_id);
"#,
    },
    Migration {
        version: 4,
        name: "handoff_protocol",
        sql: r#"
ALTER TABLE handoffs ADD COLUMN lease_id TEXT;
ALTER TABLE handoffs ADD COLUMN source_generation TEXT;
ALTER TABLE handoffs ADD COLUMN committed_at_unix_seconds INTEGER;
ALTER TABLE handoffs ADD COLUMN aborted_at_unix_seconds INTEGER;

CREATE INDEX idx_handoffs_lease_state
    ON handoffs(lease_id, state);
"#,
    },
];

pub struct MetadataDb {
    conn: Connection,
}

pub struct CanonicalPublishRequest<'a> {
    pub lease_id: &'a str,
    pub session_id: &'a str,
    pub expected_epoch: u64,
    pub holder_device_id: &'a str,
    pub expected_latest_snapshot_id: Option<&'a str>,
    pub metadata: &'a SnapshotMetadata,
    pub pinned: bool,
    pub label: Option<&'a str>,
}

#[derive(Debug)]
pub struct CanonicalPublishResult {
    pub snapshot: StoredSnapshot,
    pub latest_snapshot_id: String,
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

    pub fn ensure_default_session(
        &self,
        project_id: &str,
        display_name: &str,
        active_workspace_id: Option<&str>,
    ) -> Result<StoredSession> {
        self.conn.execute(
            "INSERT OR IGNORE INTO projects (project_id, display_name) VALUES (?1, ?2)",
            (project_id, display_name),
        )?;
        self.conn.execute(
            "UPDATE projects SET display_name = ?1 WHERE project_id = ?2",
            (display_name, project_id),
        )?;

        if let Some(session) = self
            .conn
            .query_row(
                r#"
SELECT session_id,
       project_id,
       name,
       parent_session_id,
       active_workspace_id,
       state,
       archived_at_unix_seconds,
       created_at_unix_seconds,
       updated_at_unix_seconds
FROM sessions
WHERE project_id = ?1 AND parent_session_id IS NULL
ORDER BY created_at_unix_seconds ASC, session_id ASC
LIMIT 1
"#,
                [project_id],
                stored_session_from_row,
            )
            .optional()?
        {
            return Ok(session);
        }

        self.insert_session(
            project_id,
            display_name,
            None,
            active_workspace_id,
            SessionState::Active,
        )
    }

    pub fn insert_session(
        &self,
        project_id: &str,
        name: &str,
        parent_session_id: Option<&str>,
        active_workspace_id: Option<&str>,
        state: SessionState,
    ) -> Result<StoredSession> {
        let session_id = generate_session_id();
        let now = unix_now_seconds();
        self.conn.execute(
            r#"
INSERT INTO sessions (
    session_id,
    project_id,
    active_workspace_id,
    state,
    name,
    parent_session_id,
    archived_at_unix_seconds,
    created_at_unix_seconds,
    updated_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
"#,
            (
                session_id.as_str(),
                project_id,
                active_workspace_id,
                state.as_str(),
                name,
                parent_session_id,
                Option::<i64>::None,
                now as i64,
                now as i64,
            ),
        )?;
        self.get_session(&session_id)?
            .ok_or_else(|| crate::DevRelayError::Config("session insert disappeared".to_string()))
    }

    pub fn list_sessions(&self, project_id: Option<&str>) -> Result<Vec<StoredSession>> {
        let sql = r#"
SELECT session_id,
       project_id,
       name,
       parent_session_id,
       active_workspace_id,
       state,
       archived_at_unix_seconds,
       created_at_unix_seconds,
       updated_at_unix_seconds
FROM sessions
"#;
        let mut sessions = if let Some(project_id) = project_id {
            let mut statement = self.conn.prepare(&format!(
                "{sql} WHERE project_id = ?1 ORDER BY created_at_unix_seconds ASC, session_id ASC"
            ))?;
            let rows = statement.query_map([project_id], stored_session_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut statement = self.conn.prepare(&format!(
                "{sql} ORDER BY project_id ASC, created_at_unix_seconds ASC, session_id ASC"
            ))?;
            let rows = statement.query_map([], stored_session_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        sessions.sort_by(|left, right| {
            left.project_id
                .cmp(&right.project_id)
                .then(
                    left.created_at_unix_seconds
                        .cmp(&right.created_at_unix_seconds),
                )
                .then(left.session_id.cmp(&right.session_id))
        });
        Ok(sessions)
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<StoredSession>> {
        self.conn
            .query_row(
                r#"
SELECT session_id,
       project_id,
       name,
       parent_session_id,
       active_workspace_id,
       state,
       archived_at_unix_seconds,
       created_at_unix_seconds,
       updated_at_unix_seconds
FROM sessions
WHERE session_id = ?1
"#,
                [session_id],
                stored_session_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn fork_session(&self, session_id: &str, name: &str) -> Result<StoredSession> {
        let parent = self
            .get_session(session_id)?
            .ok_or_else(|| crate::DevRelayError::Config(format!("unknown session {session_id}")))?;
        self.insert_session(
            &parent.project_id,
            name,
            Some(&parent.session_id),
            parent.active_workspace_id.as_deref(),
            SessionState::Fork,
        )
    }

    pub fn archive_session(&self, session_id: &str) -> Result<StoredSession> {
        let now = unix_now_seconds();
        let changed = self.conn.execute(
            r#"
UPDATE sessions
SET state = ?1,
    archived_at_unix_seconds = ?2,
    updated_at_unix_seconds = ?2
WHERE session_id = ?3
"#,
            (SessionState::Archived.as_str(), now as i64, session_id),
        )?;
        if changed == 0 {
            return Err(crate::DevRelayError::Config(format!(
                "unknown session {session_id}"
            )));
        }
        self.get_session(session_id)?
            .ok_or_else(|| crate::DevRelayError::Config("session archive disappeared".to_string()))
    }

    pub fn upsert_lease(&self, lease: &LeaseRecord) -> Result<()> {
        self.conn.execute(
            r#"
INSERT INTO leases (
    lease_id,
    project_id,
    holder_workspace_id,
    state,
    expires_at_unix_seconds,
    session_id,
    epoch,
    holder_device_id,
    latest_snapshot_id,
    handoff_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
ON CONFLICT(lease_id) DO UPDATE SET
    project_id = excluded.project_id,
    state = excluded.state,
    session_id = excluded.session_id,
    epoch = excluded.epoch,
    holder_device_id = excluded.holder_device_id,
    latest_snapshot_id = excluded.latest_snapshot_id,
    handoff_id = excluded.handoff_id
"#,
            (
                lease.lease_id.as_str(),
                lease.project_id.as_str(),
                Option::<&str>::None,
                lease.state.as_str(),
                Option::<i64>::None,
                lease.session_id.as_str(),
                lease.epoch as i64,
                lease.holder_device_id.as_deref(),
                lease.latest_snapshot_id.as_deref(),
                lease.handoff_id.as_deref(),
            ),
        )?;
        Ok(())
    }

    pub fn get_lease(&self, lease_id: &str) -> Result<Option<LeaseRecord>> {
        self.conn
            .query_row(
                r#"
SELECT lease_id,
       project_id,
       session_id,
       state,
       epoch,
       holder_device_id,
       latest_snapshot_id,
       handoff_id
FROM leases
WHERE lease_id = ?1
"#,
                [lease_id],
                lease_record_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn publish_snapshot_canonical(
        &mut self,
        request: CanonicalPublishRequest<'_>,
    ) -> Result<CanonicalPublishResult> {
        request.metadata.validate()?;
        if request.metadata.session_id.as_deref() != Some(request.session_id) {
            return Err(crate::DevRelayError::Config(
                "snapshot session_id does not match publish session".to_string(),
            ));
        }

        let tx = self.conn.transaction()?;
        let lease = tx
            .query_row(
                r#"
SELECT lease_id,
       project_id,
       session_id,
       state,
       epoch,
       holder_device_id,
       latest_snapshot_id,
       handoff_id
FROM leases
WHERE lease_id = ?1
"#,
                [request.lease_id],
                lease_record_from_row,
            )
            .optional()?
            .ok_or_else(|| {
                crate::DevRelayError::Config(format!("unknown lease {}", request.lease_id))
            })?;

        if lease.project_id != request.metadata.project_id {
            return Err(crate::DevRelayError::Config(
                "lease project_id does not match snapshot project_id".to_string(),
            ));
        }
        if lease.session_id != request.session_id {
            return Err(crate::DevRelayError::Config(
                "lease session_id does not match publish session".to_string(),
            ));
        }
        let session_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1 AND project_id = ?2)",
            (request.session_id, request.metadata.project_id.as_str()),
            |row| row.get(0),
        )?;
        if !session_exists {
            return Err(crate::DevRelayError::Config(format!(
                "unknown session {}",
                request.session_id
            )));
        }
        if lease.holder_device_id.as_deref() != Some(request.holder_device_id) {
            return Err(crate::DevRelayError::Config(
                "publish rejected: holder device mismatch".to_string(),
            ));
        }
        if lease.state != LeaseState::Active {
            return Err(crate::DevRelayError::Config(format!(
                "publish rejected: lease state is {}",
                lease.state.as_str()
            )));
        }

        let metadata_json = serde_json::to_string(request.metadata)?;
        let sequence_number: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence_number), 0) + 1 FROM snapshots WHERE project_id = ?1",
            [request.metadata.project_id.as_str()],
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
                request.metadata.snapshot_id.as_str(),
                request.metadata.project_id.as_str(),
                request.session_id,
                request.metadata.parent_snapshot_id.as_deref(),
                sequence_number,
                request.pinned,
                request.label,
                metadata_json.as_str(),
                request.metadata.created_at_unix_seconds as i64,
            ),
        )?;

        let snapshot = StoredSnapshot {
            snapshot_id: request.metadata.snapshot_id.clone(),
            project_id: request.metadata.project_id.clone(),
            session_id: Some(request.session_id.to_string()),
            parent_snapshot_id: request.metadata.parent_snapshot_id.clone(),
            sequence_number,
            pinned: request.pinned,
            label: request.label.map(ToString::to_string),
            metadata: request.metadata.clone(),
            created_at_unix_seconds: request.metadata.created_at_unix_seconds,
        };

        let stale_error = if lease.epoch != request.expected_epoch {
            Some(stale_publish_error(
                "lease epoch changed; refresh before publishing",
            ))
        } else if lease.latest_snapshot_id.as_deref() != request.expected_latest_snapshot_id {
            Some(stale_publish_error(
                "canonical latest changed; recover or fork before publishing",
            ))
        } else {
            tx.execute(
                "UPDATE leases SET latest_snapshot_id = ?1 WHERE lease_id = ?2",
                (request.metadata.snapshot_id.as_str(), request.lease_id),
            )?;
            None
        };
        tx.commit()?;

        if let Some(err) = stale_error {
            return Err(err);
        }

        Ok(CanonicalPublishResult {
            snapshot,
            latest_snapshot_id: request.metadata.snapshot_id.clone(),
        })
    }

    pub fn begin_handoff(
        &mut self,
        lease_id: &str,
        source_device_id: &str,
        target_device_id: &str,
        source_generation: &str,
        ttl_seconds: u64,
    ) -> Result<HandoffRecord> {
        let tx = self.conn.transaction()?;
        let lease = tx
            .query_row(
                r#"
SELECT lease_id,
       project_id,
       session_id,
       state,
       epoch,
       holder_device_id,
       latest_snapshot_id,
       handoff_id
FROM leases
WHERE lease_id = ?1
"#,
                [lease_id],
                lease_record_from_row,
            )
            .optional()?
            .ok_or_else(|| crate::DevRelayError::Config(format!("unknown lease {lease_id}")))?;
        let existing: Option<String> = tx
            .query_row(
                r#"
SELECT handoff_id
FROM handoffs
WHERE lease_id = ?1 AND state NOT IN ('committed', 'aborted')
ORDER BY created_at_unix_seconds ASC, handoff_id ASC
LIMIT 1
"#,
                [lease_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            return Err(crate::DevRelayError::Config(format!(
                "handoff already pending: {existing}"
            )));
        }
        if lease.state != LeaseState::Active {
            return Err(crate::DevRelayError::Config(format!(
                "cannot begin handoff from {} lease",
                lease.state.as_str()
            )));
        }
        if lease.holder_device_id.as_deref() != Some(source_device_id) {
            return Err(crate::DevRelayError::Config(
                "handoff source does not hold lease".to_string(),
            ));
        }

        let now = unix_now_seconds();
        let expires_at = now.saturating_add(ttl_seconds.max(1));
        let handoff = HandoffRecord {
            handoff_id: generate_handoff_id(),
            lease_id: lease_id.to_string(),
            project_id: lease.project_id.clone(),
            expected_epoch: lease.epoch,
            source_device_id: source_device_id.to_string(),
            target_device_id: target_device_id.to_string(),
            source_generation: source_generation.to_string(),
            expires_at_unix_seconds: expires_at,
            state: HandoffState::TargetPrepare,
        };
        tx.execute(
            r#"
INSERT INTO handoffs (
    handoff_id,
    project_id,
    source_workspace_id,
    target_workspace_id,
    state,
    lease_id,
    expected_epoch,
    source_device_id,
    target_device_id,
    source_generation,
    expires_at_unix_seconds,
    created_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
"#,
            (
                handoff.handoff_id.as_str(),
                handoff.project_id.as_str(),
                Option::<&str>::None,
                Option::<&str>::None,
                handoff.state.as_str(),
                handoff.lease_id.as_str(),
                handoff.expected_epoch as i64,
                handoff.source_device_id.as_str(),
                handoff.target_device_id.as_str(),
                handoff.source_generation.as_str(),
                handoff.expires_at_unix_seconds as i64,
                now as i64,
            ),
        )?;
        tx.execute(
            "UPDATE leases SET state = ?1, handoff_id = ?2 WHERE lease_id = ?3",
            (
                LeaseState::HandoffPending.as_str(),
                handoff.handoff_id.as_str(),
                lease_id,
            ),
        )?;
        tx.commit()?;
        Ok(handoff)
    }

    pub fn mark_handoff_target_verified(&self, handoff_id: &str) -> Result<HandoffRecord> {
        self.update_handoff_state(
            handoff_id,
            HandoffState::TargetPrepare,
            HandoffState::TargetVerified,
        )
    }

    pub fn mark_handoff_source_ready(&self, handoff_id: &str) -> Result<HandoffRecord> {
        self.update_handoff_state(
            handoff_id,
            HandoffState::TargetVerified,
            HandoffState::SourceReady,
        )
    }

    pub fn abort_handoff(&self, handoff_id: &str) -> Result<HandoffRecord> {
        let handoff = self
            .get_handoff(handoff_id)?
            .ok_or_else(|| crate::DevRelayError::Config(format!("unknown handoff {handoff_id}")))?;
        self.conn.execute(
            "UPDATE handoffs SET state = ?1, aborted_at_unix_seconds = ?2 WHERE handoff_id = ?3",
            (
                HandoffState::Aborted.as_str(),
                unix_now_seconds() as i64,
                handoff_id,
            ),
        )?;
        self.conn.execute(
            "UPDATE leases SET state = ?1, handoff_id = NULL WHERE lease_id = ?2 AND handoff_id = ?3",
            (LeaseState::Active.as_str(), handoff.lease_id.as_str(), handoff_id),
        )?;
        self.get_handoff(handoff_id)?
            .ok_or_else(|| crate::DevRelayError::Config("handoff abort disappeared".to_string()))
    }

    pub fn commit_handoff(
        &mut self,
        handoff_id: &str,
        observed_source_generation: &str,
        now_unix_seconds: u64,
    ) -> Result<HandoffRecord> {
        let tx = self.conn.transaction()?;
        let handoff = tx
            .query_row(
                handoff_select_sql("WHERE handoff_id = ?1").as_str(),
                [handoff_id],
                handoff_record_from_row,
            )
            .optional()?
            .ok_or_else(|| crate::DevRelayError::Config(format!("unknown handoff {handoff_id}")))?;
        if handoff.state != HandoffState::SourceReady {
            return Err(crate::DevRelayError::Config(format!(
                "handoff is not source-ready: {}",
                handoff.state.as_str()
            )));
        }
        let abort_reason = if now_unix_seconds > handoff.expires_at_unix_seconds {
            Some("handoff expired")
        } else if observed_source_generation != handoff.source_generation {
            Some("source generation changed")
        } else {
            None
        };
        if let Some(reason) = abort_reason {
            tx.execute(
                "UPDATE handoffs SET state = ?1, aborted_at_unix_seconds = ?2 WHERE handoff_id = ?3",
                (HandoffState::Aborted.as_str(), now_unix_seconds as i64, handoff_id),
            )?;
            tx.execute(
                "UPDATE leases SET state = ?1, handoff_id = NULL WHERE lease_id = ?2 AND handoff_id = ?3",
                (LeaseState::Active.as_str(), handoff.lease_id.as_str(), handoff_id),
            )?;
            tx.commit()?;
            return Err(crate::DevRelayError::Config(format!(
                "handoff commit rejected: {reason}"
            )));
        }

        tx.execute(
            r#"
UPDATE leases
SET state = ?1,
    epoch = ?2,
    holder_device_id = ?3,
    handoff_id = NULL
WHERE lease_id = ?4 AND epoch = ?5 AND handoff_id = ?6
"#,
            (
                LeaseState::Active.as_str(),
                handoff.expected_epoch.saturating_add(1) as i64,
                handoff.target_device_id.as_str(),
                handoff.lease_id.as_str(),
                handoff.expected_epoch as i64,
                handoff_id,
            ),
        )?;
        tx.execute(
            "UPDATE handoffs SET state = ?1, committed_at_unix_seconds = ?2 WHERE handoff_id = ?3",
            (
                HandoffState::Committed.as_str(),
                now_unix_seconds as i64,
                handoff_id,
            ),
        )?;
        tx.commit()?;
        self.get_handoff(handoff_id)?
            .ok_or_else(|| crate::DevRelayError::Config("handoff commit disappeared".to_string()))
    }

    pub fn get_handoff(&self, handoff_id: &str) -> Result<Option<HandoffRecord>> {
        self.conn
            .query_row(
                handoff_select_sql("WHERE handoff_id = ?1").as_str(),
                [handoff_id],
                handoff_record_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn update_handoff_state(
        &self,
        handoff_id: &str,
        expected: HandoffState,
        next: HandoffState,
    ) -> Result<HandoffRecord> {
        let changed = self.conn.execute(
            "UPDATE handoffs SET state = ?1 WHERE handoff_id = ?2 AND state = ?3",
            (next.as_str(), handoff_id, expected.as_str()),
        )?;
        if changed == 0 {
            return Err(crate::DevRelayError::Config(format!(
                "handoff {handoff_id} is not {}",
                expected.as_str()
            )));
        }
        self.get_handoff(handoff_id)?
            .ok_or_else(|| crate::DevRelayError::Config("handoff update disappeared".to_string()))
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

fn stored_session_from_row(row: &Row<'_>) -> rusqlite::Result<StoredSession> {
    let archived_at_unix_seconds = row
        .get::<_, Option<i64>>(6)?
        .map(|value| value.max(0) as u64);
    Ok(StoredSession {
        session_id: row.get(0)?,
        project_id: row.get(1)?,
        name: row
            .get::<_, Option<String>>(2)?
            .unwrap_or_else(|| "Default".to_string()),
        parent_session_id: row.get(3)?,
        active_workspace_id: row.get(4)?,
        state: SessionState::parse(&row.get::<_, String>(5)?),
        archived_at_unix_seconds,
        created_at_unix_seconds: row.get::<_, i64>(7)?.max(0) as u64,
        updated_at_unix_seconds: row.get::<_, i64>(8)?.max(0) as u64,
    })
}

fn lease_record_from_row(row: &Row<'_>) -> rusqlite::Result<LeaseRecord> {
    Ok(LeaseRecord {
        lease_id: row.get(0)?,
        project_id: row.get(1)?,
        session_id: row
            .get::<_, Option<String>>(2)?
            .unwrap_or_else(|| "unknown-session".to_string()),
        state: parse_lease_state(&row.get::<_, String>(3)?),
        epoch: row.get::<_, i64>(4)?.max(0) as u64,
        holder_device_id: row.get(5)?,
        latest_snapshot_id: row.get(6)?,
        handoff_id: row.get(7)?,
    })
}

fn parse_lease_state(value: &str) -> LeaseState {
    match value {
        "handoff-pending" => LeaseState::HandoffPending,
        "committing" => LeaseState::Committing,
        "inactive" => LeaseState::Inactive,
        "forked" => LeaseState::Forked,
        "archived" => LeaseState::Archived,
        _ => LeaseState::Active,
    }
}

fn stale_publish_error(detail: &str) -> crate::DevRelayError {
    crate::DevRelayError::Config(format!(
        "stale publish: {detail}; safe action: refresh lease"
    ))
}

fn handoff_select_sql(where_clause: &str) -> String {
    format!(
        r#"
SELECT handoff_id,
       lease_id,
       project_id,
       expected_epoch,
       source_device_id,
       target_device_id,
       source_generation,
       expires_at_unix_seconds,
       state
FROM handoffs
{where_clause}
"#
    )
}

fn handoff_record_from_row(row: &Row<'_>) -> rusqlite::Result<HandoffRecord> {
    Ok(HandoffRecord {
        handoff_id: row.get(0)?,
        lease_id: row
            .get::<_, Option<String>>(1)?
            .unwrap_or_else(|| "unknown-lease".to_string()),
        project_id: row.get(2)?,
        expected_epoch: row.get::<_, Option<i64>>(3)?.unwrap_or_default().max(0) as u64,
        source_device_id: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        target_device_id: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        source_generation: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        expires_at_unix_seconds: row.get::<_, Option<i64>>(7)?.unwrap_or_default().max(0) as u64,
        state: HandoffState::parse(&row.get::<_, String>(8)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalConfig, SnapshotMetadata};

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
        assert!(index_exists(&db, "idx_sessions_project_state"));
        assert!(index_exists(&db, "idx_sessions_parent"));
        assert!(index_exists(&db, "idx_handoffs_project_state"));
        assert!(index_exists(&db, "idx_handoffs_source_device_state"));
        assert!(index_exists(&db, "idx_handoffs_target_device_state"));
        assert!(index_exists(&db, "idx_handoffs_lease_state"));

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
        for column in [
            "session_id",
            "project_id",
            "name",
            "parent_session_id",
            "archived_at_unix_seconds",
        ] {
            assert!(column_exists(&db, "sessions", column), "{column}");
        }
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
            "lease_id",
            "source_device_id",
            "target_device_id",
            "source_generation",
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
    fn stores_and_updates_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();

        let default = db
            .ensure_default_session("project123", "Demo Project", None)
            .unwrap();
        assert!(default.session_id.starts_with("se_"));
        assert_eq!(default.project_id, "project123");
        assert_eq!(default.name, "Demo Project");
        assert_eq!(default.parent_session_id, None);
        assert_eq!(default.active_workspace_id, None);
        assert_eq!(default.state, SessionState::Active);
        assert_eq!(default.archived_at_unix_seconds, None);

        let same = db
            .ensure_default_session("project123", "Renamed Project", None)
            .unwrap();
        assert_eq!(same.session_id, default.session_id);

        let fork = db.fork_session(&default.session_id, "Experiment").unwrap();
        assert!(fork.session_id.starts_with("se_"));
        assert_eq!(
            fork.parent_session_id.as_deref(),
            Some(default.session_id.as_str())
        );
        assert_eq!(fork.active_workspace_id, default.active_workspace_id);
        assert_eq!(fork.state, SessionState::Fork);

        let sessions = db.list_sessions(Some("project123")).unwrap();
        assert_eq!(sessions.len(), 2);

        let archived = db.archive_session(&fork.session_id).unwrap();
        assert_eq!(archived.state, SessionState::Archived);
        assert!(archived.archived_at_unix_seconds.is_some());
    }

    fn publish_metadata(snapshot_id: &str, session_id: &str) -> SnapshotMetadata {
        let mut metadata: SnapshotMetadata =
            serde_json::from_str(include_str!("../tests/fixtures/snapshot_metadata_v1.json"))
                .unwrap();
        metadata.project_id = "project123".to_string();
        metadata.project_name = "Demo Project".to_string();
        metadata.session_id = Some(session_id.to_string());
        metadata.snapshot_id = snapshot_id.to_string();
        metadata.parent_snapshot_id = None;
        metadata
    }

    fn setup_publish_db(epoch: u64, state: LeaseState) -> (MetadataDb, StoredSession, LeaseRecord) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.keep().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();
        let session = db
            .ensure_default_session("project123", "Demo Project", None)
            .unwrap();
        let lease = LeaseRecord {
            lease_id: "lease-1".to_string(),
            project_id: "project123".to_string(),
            session_id: session.session_id.clone(),
            state,
            epoch,
            holder_device_id: Some("device-a".to_string()),
            latest_snapshot_id: None,
            handoff_id: None,
        };
        db.upsert_lease(&lease).unwrap();
        (db, session, lease)
    }

    #[test]
    fn canonical_publish_persists_snapshot_and_advances_latest() {
        let (mut db, session, lease) = setup_publish_db(1, LeaseState::Active);
        let metadata = publish_metadata("s1_000000000000000000000101", &session.session_id);

        let result = db
            .publish_snapshot_canonical(CanonicalPublishRequest {
                lease_id: &lease.lease_id,
                session_id: &session.session_id,
                expected_epoch: 1,
                holder_device_id: "device-a",
                expected_latest_snapshot_id: None,
                metadata: &metadata,
                pinned: false,
                label: Some("canonical"),
            })
            .unwrap();

        assert_eq!(result.snapshot.snapshot_id, metadata.snapshot_id);
        assert_eq!(result.snapshot.sequence_number, 1);
        assert_eq!(result.latest_snapshot_id, metadata.snapshot_id);
        let updated = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(
            updated.latest_snapshot_id.as_deref(),
            Some(metadata.snapshot_id.as_str())
        );
    }

    #[test]
    fn stale_epoch_preserves_snapshot_without_advancing_latest() {
        let (mut db, session, lease) = setup_publish_db(2, LeaseState::Active);
        let metadata = publish_metadata("s1_000000000000000000000102", &session.session_id);

        let err = db
            .publish_snapshot_canonical(CanonicalPublishRequest {
                lease_id: &lease.lease_id,
                session_id: &session.session_id,
                expected_epoch: 1,
                holder_device_id: "device-a",
                expected_latest_snapshot_id: None,
                metadata: &metadata,
                pinned: true,
                label: Some("stale"),
            })
            .unwrap_err();

        assert!(err.to_string().contains("stale publish"));
        assert_eq!(db.list_sessions(Some("project123")).unwrap().len(), 1);
        let snapshots = snapshots_for_project(&db, "project123");
        assert_eq!(snapshots, vec![metadata.snapshot_id.clone()]);
        let updated = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(updated.latest_snapshot_id, None);
    }

    #[test]
    fn wrong_holder_and_inactive_lease_reject_publish() {
        let (mut db, session, lease) = setup_publish_db(1, LeaseState::Active);
        let metadata = publish_metadata("s1_000000000000000000000103", &session.session_id);
        let err = db
            .publish_snapshot_canonical(CanonicalPublishRequest {
                lease_id: &lease.lease_id,
                session_id: &session.session_id,
                expected_epoch: 1,
                holder_device_id: "device-b",
                expected_latest_snapshot_id: None,
                metadata: &metadata,
                pinned: false,
                label: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("holder device mismatch"));
        assert!(snapshots_for_project(&db, "project123").is_empty());

        let (mut db, session, lease) = setup_publish_db(1, LeaseState::Inactive);
        let metadata = publish_metadata("s1_000000000000000000000104", &session.session_id);
        let err = db
            .publish_snapshot_canonical(CanonicalPublishRequest {
                lease_id: &lease.lease_id,
                session_id: &session.session_id,
                expected_epoch: 1,
                holder_device_id: "device-a",
                expected_latest_snapshot_id: None,
                metadata: &metadata,
                pinned: false,
                label: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("lease state is inactive"));
        assert!(snapshots_for_project(&db, "project123").is_empty());
    }

    #[test]
    fn concurrent_publish_preserves_stale_snapshot_without_latest_change() {
        let (mut db, session, lease) = setup_publish_db(1, LeaseState::Active);
        let first = publish_metadata("s1_000000000000000000000105", &session.session_id);
        let second = publish_metadata("s1_000000000000000000000106", &session.session_id);

        db.publish_snapshot_canonical(CanonicalPublishRequest {
            lease_id: &lease.lease_id,
            session_id: &session.session_id,
            expected_epoch: 1,
            holder_device_id: "device-a",
            expected_latest_snapshot_id: None,
            metadata: &first,
            pinned: false,
            label: Some("first"),
        })
        .unwrap();
        let err = db
            .publish_snapshot_canonical(CanonicalPublishRequest {
                lease_id: &lease.lease_id,
                session_id: &session.session_id,
                expected_epoch: 1,
                holder_device_id: "device-a",
                expected_latest_snapshot_id: None,
                metadata: &second,
                pinned: false,
                label: Some("second"),
            })
            .unwrap_err();

        assert!(err.to_string().contains("canonical latest changed"));
        assert_eq!(
            snapshots_for_project(&db, "project123"),
            vec![first.snapshot_id.clone(), second.snapshot_id.clone()]
        );
        let updated = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(
            updated.latest_snapshot_id.as_deref(),
            Some(first.snapshot_id.as_str())
        );
    }

    #[test]
    fn handoff_happy_path_commits_holder_and_epoch() {
        let (mut db, _session, lease) = setup_publish_db(7, LeaseState::Active);

        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
            .unwrap();
        assert!(handoff.handoff_id.starts_with("ho_"));
        assert_eq!(handoff.expected_epoch, 7);
        assert_eq!(handoff.source_device_id, "device-a");
        assert_eq!(handoff.target_device_id, "device-b");
        assert_eq!(handoff.source_generation, "gen-1");
        assert_eq!(handoff.state, HandoffState::TargetPrepare);

        let pending = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(pending.state, LeaseState::HandoffPending);
        assert_eq!(
            pending.handoff_id.as_deref(),
            Some(handoff.handoff_id.as_str())
        );

        db.mark_handoff_target_verified(&handoff.handoff_id)
            .unwrap();
        db.mark_handoff_source_ready(&handoff.handoff_id).unwrap();
        let committed = db
            .commit_handoff(
                &handoff.handoff_id,
                "gen-1",
                handoff.expires_at_unix_seconds - 1,
            )
            .unwrap();
        assert_eq!(committed.state, HandoffState::Committed);

        let updated = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(updated.state, LeaseState::Active);
        assert_eq!(updated.epoch, 8);
        assert_eq!(updated.holder_device_id.as_deref(), Some("device-b"));
        assert_eq!(updated.handoff_id, None);
    }

    #[test]
    fn handoff_source_change_aborts_without_holder_change() {
        let (mut db, _session, lease) = setup_publish_db(3, LeaseState::Active);
        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
            .unwrap();
        db.mark_handoff_target_verified(&handoff.handoff_id)
            .unwrap();
        db.mark_handoff_source_ready(&handoff.handoff_id).unwrap();

        let err = db
            .commit_handoff(
                &handoff.handoff_id,
                "gen-2",
                handoff.expires_at_unix_seconds - 1,
            )
            .unwrap_err();
        assert!(err.to_string().contains("source generation changed"));
        assert_eq!(
            db.get_handoff(&handoff.handoff_id).unwrap().unwrap().state,
            HandoffState::Aborted
        );
        let updated = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(updated.state, LeaseState::Active);
        assert_eq!(updated.epoch, 3);
        assert_eq!(updated.holder_device_id.as_deref(), Some("device-a"));
    }

    #[test]
    fn handoff_target_apply_failure_aborts() {
        let (mut db, _session, lease) = setup_publish_db(4, LeaseState::Active);
        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
            .unwrap();

        let aborted = db.abort_handoff(&handoff.handoff_id).unwrap();
        assert_eq!(aborted.state, HandoffState::Aborted);
        let updated = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(updated.state, LeaseState::Active);
        assert_eq!(updated.epoch, 4);
        assert_eq!(updated.holder_device_id.as_deref(), Some("device-a"));
        assert_eq!(updated.handoff_id, None);
    }

    #[test]
    fn handoff_commit_rejects_expired_handoff() {
        let (mut db, _session, lease) = setup_publish_db(5, LeaseState::Active);
        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 1)
            .unwrap();
        db.mark_handoff_target_verified(&handoff.handoff_id)
            .unwrap();
        db.mark_handoff_source_ready(&handoff.handoff_id).unwrap();

        let err = db
            .commit_handoff(
                &handoff.handoff_id,
                "gen-1",
                handoff.expires_at_unix_seconds + 1,
            )
            .unwrap_err();
        assert!(err.to_string().contains("handoff expired"));
        assert_eq!(
            db.get_handoff(&handoff.handoff_id).unwrap().unwrap().state,
            HandoffState::Aborted
        );
        let updated = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(updated.epoch, 5);
        assert_eq!(updated.holder_device_id.as_deref(), Some("device-a"));
    }

    #[test]
    fn concurrent_handoff_attempt_is_rejected_deterministically() {
        let (mut db, _session, lease) = setup_publish_db(6, LeaseState::Active);
        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
            .unwrap();

        let err = db
            .begin_handoff(&lease.lease_id, "device-a", "device-c", "gen-1", 60)
            .unwrap_err();
        assert!(err.to_string().contains("handoff already pending"));
        assert_eq!(
            db.get_handoff(&handoff.handoff_id).unwrap().unwrap().state,
            HandoffState::TargetPrepare
        );
    }

    fn snapshots_for_project(db: &MetadataDb, project_id: &str) -> Vec<String> {
        let mut statement = db
            .connection()
            .prepare(
                "SELECT snapshot_id FROM snapshots WHERE project_id = ?1 ORDER BY sequence_number",
            )
            .unwrap();
        let rows = statement
            .query_map([project_id], |row| row.get::<_, String>(0))
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
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
        assert_eq!(count, 4);
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
