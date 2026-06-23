//! SQLite metadata storage and migrations.
//!
//! M1 stores local registry, workspace, session, snapshot, lease, and handoff
//! metadata in a per-project SQLite database. Migrations are monotonic and run
//! inside a transaction so a failed migration leaves the previous schema intact.

use crate::{
    AUDIT_SCHEMA_VERSION, AuditEventInput, AuditEventRecord, AuditEventType, AuditOutcome,
    CasStore, DeviceIdentity, DevicePublicIdentity, DeviceRevocationRecord, FabricRootIdentity,
    HandoffJournalPhase, HandoffJournalRecord, HandoffRecord, HandoffRecoveryOutcome, HandoffState,
    LeaseRecord, LeaseState, PairingSession, PairingState, Result, SessionState, SnapshotMetadata,
    StoredSession, StoredSnapshot, compute_handshake_transcript_hash,
    derive_short_authentication_string, ensure_sidecars_available, generate_ephemeral_pairing_key,
    generate_handoff_id, generate_pairing_id, generate_session_id, unix_now_seconds,
    validate_key_hex,
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
    Migration {
        version: 5,
        name: "handoff_journal",
        sql: r#"
CREATE TABLE handoff_journal (
    journal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    handoff_id TEXT NOT NULL,
    lease_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    phase TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    created_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch()),
    FOREIGN KEY(handoff_id) REFERENCES handoffs(handoff_id),
    FOREIGN KEY(project_id) REFERENCES projects(project_id)
);

CREATE INDEX idx_handoff_journal_handoff
    ON handoff_journal(handoff_id, journal_id);
CREATE INDEX idx_handoff_journal_project_phase
    ON handoff_journal(project_id, phase, created_at_unix_seconds);
"#,
    },
    Migration {
        version: 6,
        name: "fabric_identity",
        sql: r#"
CREATE TABLE fabric_roots (
    fabric_id TEXT PRIMARY KEY,
    fabric_name TEXT NOT NULL,
    root_public_key_hex TEXT NOT NULL,
    created_at_unix_seconds INTEGER NOT NULL,
    rotation_epoch INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE device_public_identities (
    device_id TEXT PRIMARY KEY,
    fabric_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    signing_public_key_hex TEXT NOT NULL,
    network_public_key_hex TEXT NOT NULL,
    platform_key TEXT NOT NULL,
    architecture TEXT NOT NULL,
    created_at_unix_seconds INTEGER NOT NULL,
    last_seen_unix_seconds INTEGER NOT NULL,
    FOREIGN KEY(fabric_id) REFERENCES fabric_roots(fabric_id)
);

CREATE INDEX idx_device_public_identities_fabric
    ON device_public_identities(fabric_id, device_id);
"#,
    },
    Migration {
        version: 7,
        name: "pairing_sessions",
        sql: r#"
CREATE TABLE pairing_sessions (
    pairing_id TEXT PRIMARY KEY,
    fabric_id TEXT NOT NULL,
    local_device_id TEXT NOT NULL,
    peer_device_id TEXT NOT NULL,
    peer_display_name TEXT NOT NULL,
    peer_signing_public_key_hex TEXT NOT NULL,
    peer_network_public_key_hex TEXT NOT NULL,
    anchor_address TEXT,
    local_ephemeral_public_key_hex TEXT NOT NULL,
    peer_ephemeral_public_key_hex TEXT NOT NULL,
    transcript_hash_hex TEXT NOT NULL,
    short_authentication_string TEXT NOT NULL,
    state TEXT NOT NULL,
    certificate_json TEXT,
    expires_at_unix_seconds INTEGER NOT NULL,
    confirmed_at_unix_seconds INTEGER,
    aborted_at_unix_seconds INTEGER,
    created_at_unix_seconds INTEGER NOT NULL,
    FOREIGN KEY(fabric_id) REFERENCES fabric_roots(fabric_id)
);

CREATE INDEX idx_pairing_sessions_fabric_state
    ON pairing_sessions(fabric_id, state, expires_at_unix_seconds);
CREATE INDEX idx_pairing_sessions_peer
    ON pairing_sessions(peer_device_id, state);
"#,
    },
    Migration {
        version: 8,
        name: "audit_events",
        sql: r#"
CREATE TABLE audit_events (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type TEXT NOT NULL,
    project_id TEXT,
    actor_device_id TEXT,
    target_device_id TEXT,
    session_id TEXT,
    snapshot_id TEXT,
    lease_id TEXT,
    handoff_id TEXT,
    outcome TEXT NOT NULL,
    summary TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    created_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX idx_audit_events_timeline
    ON audit_events(created_at_unix_seconds DESC, audit_id DESC);
CREATE INDEX idx_audit_events_project_timeline
    ON audit_events(project_id, created_at_unix_seconds DESC, audit_id DESC);
CREATE INDEX idx_audit_events_type_timeline
    ON audit_events(event_type, created_at_unix_seconds DESC, audit_id DESC);
CREATE INDEX idx_audit_events_snapshot
    ON audit_events(project_id, snapshot_id);
CREATE INDEX idx_audit_events_lease
    ON audit_events(project_id, lease_id);
"#,
    },
    Migration {
        version: 9,
        name: "device_revocations",
        sql: r#"
CREATE TABLE device_revocations (
    device_id TEXT PRIMARY KEY,
    revoked_by_device_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    key_rotation_required INTEGER NOT NULL DEFAULT 0,
    revoked_at_unix_seconds INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX idx_device_revocations_revoked_at
    ON device_revocations(revoked_at_unix_seconds DESC, device_id);
"#,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataDbFaultPoint {
    DuringLeaseCommit,
}

impl MetadataDbFaultPoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::DuringLeaseCommit => "during-lease-commit",
        }
    }
}

pub struct MetadataDb {
    conn: Connection,
    fault: Option<MetadataDbFaultPoint>,
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

pub struct InactiveForkPublishRequest<'a> {
    pub lease_id: &'a str,
    pub session_id: &'a str,
    pub holder_device_id: &'a str,
    pub metadata: &'a SnapshotMetadata,
    pub label: Option<&'a str>,
}

#[derive(Debug)]
pub struct InactiveForkPublishResult {
    pub fork_session: StoredSession,
    pub snapshot: StoredSnapshot,
    pub canonical_latest_snapshot_id: Option<String>,
}

pub struct PairingStartRequest<'a> {
    pub fabric_id: &'a str,
    pub local_device_id: &'a str,
    pub peer_device_id: &'a str,
    pub peer_display_name: &'a str,
    pub peer_signing_public_key_hex: &'a str,
    pub peer_network_public_key_hex: &'a str,
    pub peer_ephemeral_public_key_hex: &'a str,
    pub anchor_address: Option<&'a str>,
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct HandoffCommitSnapshotPreflight<'a> {
    pub target_repo_root: &'a Path,
    pub snapshot: &'a SnapshotMetadata,
    pub cas_store: &'a CasStore,
}

impl HandoffCommitSnapshotPreflight<'_> {
    fn verify(self, handoff: &HandoffRecord) -> Result<()> {
        if self.snapshot.project_id != handoff.project_id {
            return Err(crate::DevRelayError::Config(format!(
                "handoff snapshot project mismatch: expected {}, got {}",
                handoff.project_id, self.snapshot.project_id
            )));
        }
        ensure_sidecars_available(
            self.target_repo_root,
            &self.snapshot.sidecars,
            self.cas_store,
        )
    }
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

        let mut db = Self { conn, fault: None };
        db.run_migrations()?;
        Ok(db)
    }

    pub fn with_fault_injection(mut self, fault: MetadataDbFaultPoint) -> Self {
        self.fault = Some(fault);
        self
    }

    pub fn set_fault_injection(&mut self, fault: Option<MetadataDbFaultPoint>) {
        self.fault = fault;
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

    pub fn record_audit_event(&self, input: AuditEventInput) -> Result<AuditEventRecord> {
        self.record_audit_event_at(input, unix_now_seconds())
    }

    pub fn record_audit_event_at(
        &self,
        input: AuditEventInput,
        created_at_unix_seconds: u64,
    ) -> Result<AuditEventRecord> {
        validate_audit_input(&input)?;
        let detail_json = normalize_audit_detail(&input.detail)?;
        self.conn.execute(
            r#"
INSERT INTO audit_events (
    event_type,
    project_id,
    actor_device_id,
    target_device_id,
    session_id,
    snapshot_id,
    lease_id,
    handoff_id,
    outcome,
    summary,
    detail_json,
    created_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
"#,
            (
                input.event_type.as_str(),
                input.project_id.as_deref(),
                input.actor_device_id.as_deref(),
                input.target_device_id.as_deref(),
                input.session_id.as_deref(),
                input.snapshot_id.as_deref(),
                input.lease_id.as_deref(),
                input.handoff_id.as_deref(),
                input.outcome.as_str(),
                input.summary.as_str(),
                detail_json.as_str(),
                created_at_unix_seconds as i64,
            ),
        )?;
        Ok(AuditEventRecord {
            schema_version: AUDIT_SCHEMA_VERSION,
            audit_id: self.conn.last_insert_rowid(),
            event_type: input.event_type,
            project_id: input.project_id,
            actor_device_id: input.actor_device_id,
            target_device_id: input.target_device_id,
            session_id: input.session_id,
            snapshot_id: input.snapshot_id,
            lease_id: input.lease_id,
            handoff_id: input.handoff_id,
            outcome: input.outcome,
            summary: input.summary,
            detail: serde_json::from_str(&detail_json)?,
            created_at_unix_seconds,
        })
    }

    pub fn list_audit_events(
        &self,
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AuditEventRecord>> {
        let limit = limit.clamp(1, 1_000) as i64;
        let sql = r#"
SELECT audit_id,
       event_type,
       project_id,
       actor_device_id,
       target_device_id,
       session_id,
       snapshot_id,
       lease_id,
       handoff_id,
       outcome,
       summary,
       detail_json,
       created_at_unix_seconds
FROM audit_events
"#;
        let mut events = if let Some(project_id) = project_id {
            let mut statement = self.conn.prepare(&format!(
                "{sql} WHERE project_id = ?1 ORDER BY created_at_unix_seconds DESC, audit_id DESC LIMIT ?2"
            ))?;
            let rows = statement.query_map((project_id, limit), audit_event_record_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut statement = self.conn.prepare(&format!(
                "{sql} ORDER BY created_at_unix_seconds DESC, audit_id DESC LIMIT ?1"
            ))?;
            let rows = statement.query_map([limit], audit_event_record_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        events.sort_by(|left, right| {
            right
                .created_at_unix_seconds
                .cmp(&left.created_at_unix_seconds)
                .then(right.audit_id.cmp(&left.audit_id))
        });
        Ok(events)
    }

    pub fn revoke_device(
        &mut self,
        device_id: &str,
        revoked_by_device_id: &str,
        reason: &str,
        key_rotation_required: bool,
    ) -> Result<DeviceRevocationRecord> {
        self.revoke_device_at(
            device_id,
            revoked_by_device_id,
            reason,
            key_rotation_required,
            unix_now_seconds(),
        )
    }

    pub fn revoke_device_at(
        &mut self,
        device_id: &str,
        revoked_by_device_id: &str,
        reason: &str,
        key_rotation_required: bool,
        revoked_at_unix_seconds: u64,
    ) -> Result<DeviceRevocationRecord> {
        validate_non_empty("device_id", device_id)?;
        validate_non_empty("revoked_by_device_id", revoked_by_device_id)?;
        validate_non_empty("reason", reason)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
INSERT INTO device_revocations (
    device_id,
    revoked_by_device_id,
    reason,
    key_rotation_required,
    revoked_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(device_id) DO UPDATE SET
    revoked_by_device_id = excluded.revoked_by_device_id,
    reason = excluded.reason,
    key_rotation_required = excluded.key_rotation_required,
    revoked_at_unix_seconds = excluded.revoked_at_unix_seconds
"#,
            (
                device_id,
                revoked_by_device_id,
                reason,
                key_rotation_required,
                revoked_at_unix_seconds as i64,
            ),
        )?;
        let mut audit = AuditEventInput::new(
            AuditEventType::DeviceRevoked,
            AuditOutcome::Succeeded,
            format!("revoked device {device_id}"),
        )
        .with_detail(serde_json::json!({
            "reason": reason,
            "key_rotation_required": key_rotation_required,
        }));
        audit.actor_device_id = Some(revoked_by_device_id.to_string());
        audit.target_device_id = Some(device_id.to_string());
        insert_audit_event_tx(&tx, audit, revoked_at_unix_seconds)?;
        tx.commit()?;
        Ok(DeviceRevocationRecord {
            device_id: device_id.to_string(),
            revoked_by_device_id: revoked_by_device_id.to_string(),
            reason: reason.to_string(),
            key_rotation_required,
            revoked_at_unix_seconds,
        })
    }

    pub fn get_device_revocation(&self, device_id: &str) -> Result<Option<DeviceRevocationRecord>> {
        self.conn
            .query_row(
                r#"
SELECT device_id,
       revoked_by_device_id,
       reason,
       key_rotation_required,
       revoked_at_unix_seconds
FROM device_revocations
WHERE device_id = ?1
"#,
                [device_id],
                device_revocation_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_device_revocations(&self) -> Result<Vec<DeviceRevocationRecord>> {
        let mut statement = self.conn.prepare(
            r#"
SELECT device_id,
       revoked_by_device_id,
       reason,
       key_rotation_required,
       revoked_at_unix_seconds
FROM device_revocations
ORDER BY revoked_at_unix_seconds DESC, device_id ASC
"#,
        )?;
        let rows = statement.query_map([], device_revocation_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn ensure_device_not_revoked(&self, device_id: &str, operation: &str) -> Result<()> {
        ensure_device_not_revoked_conn(&self.conn, device_id, operation)
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

    pub fn upsert_fabric_root_identity(&self, root: &FabricRootIdentity) -> Result<()> {
        self.conn.execute(
            r#"
INSERT INTO fabric_roots (
    fabric_id,
    fabric_name,
    root_public_key_hex,
    created_at_unix_seconds,
    rotation_epoch
) VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(fabric_id) DO UPDATE SET
    fabric_name = excluded.fabric_name,
    root_public_key_hex = excluded.root_public_key_hex,
    rotation_epoch = excluded.rotation_epoch
"#,
            (
                root.fabric_id.as_str(),
                root.fabric_name.as_str(),
                root.root_public_key_hex.as_str(),
                root.created_at_unix_seconds as i64,
                root.rotation_epoch as i64,
            ),
        )?;
        Ok(())
    }

    pub fn get_fabric_root_identity(&self, fabric_id: &str) -> Result<Option<FabricRootIdentity>> {
        self.conn
            .query_row(
                r#"
SELECT fabric_id,
       fabric_name,
       root_public_key_hex,
       created_at_unix_seconds,
       rotation_epoch
FROM fabric_roots
WHERE fabric_id = ?1
"#,
                [fabric_id],
                fabric_root_identity_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn upsert_device_public_identity(&self, device: &DevicePublicIdentity) -> Result<()> {
        self.conn.execute(
            r#"
INSERT INTO device_public_identities (
    device_id,
    fabric_id,
    display_name,
    signing_public_key_hex,
    network_public_key_hex,
    platform_key,
    architecture,
    created_at_unix_seconds,
    last_seen_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT(device_id) DO UPDATE SET
    fabric_id = excluded.fabric_id,
    display_name = excluded.display_name,
    signing_public_key_hex = excluded.signing_public_key_hex,
    network_public_key_hex = excluded.network_public_key_hex,
    platform_key = excluded.platform_key,
    architecture = excluded.architecture,
    last_seen_unix_seconds = excluded.last_seen_unix_seconds
"#,
            (
                device.device_id.as_str(),
                device.fabric_id.as_str(),
                device.display_name.as_str(),
                device.signing_public_key_hex.as_str(),
                device.network_public_key_hex.as_str(),
                device.platform_key.as_str(),
                device.architecture.as_str(),
                device.created_at_unix_seconds as i64,
                device.last_seen_unix_seconds as i64,
            ),
        )?;
        Ok(())
    }

    pub fn get_device_public_identity(
        &self,
        device_id: &str,
    ) -> Result<Option<DevicePublicIdentity>> {
        self.conn
            .query_row(
                r#"
SELECT device_id,
       display_name,
       fabric_id,
       signing_public_key_hex,
       network_public_key_hex,
       platform_key,
       architecture,
       created_at_unix_seconds,
       last_seen_unix_seconds
FROM device_public_identities
WHERE device_id = ?1
"#,
                [device_id],
                device_public_identity_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn start_pairing_session(
        &mut self,
        request: PairingStartRequest<'_>,
    ) -> Result<PairingSession> {
        validate_non_empty("fabric_id", request.fabric_id)?;
        validate_non_empty("local_device_id", request.local_device_id)?;
        validate_non_empty("peer_device_id", request.peer_device_id)?;
        validate_non_empty("peer_display_name", request.peer_display_name)?;
        validate_key_hex(
            "peer_signing_public_key_hex",
            request.peer_signing_public_key_hex,
        )?;
        validate_key_hex(
            "peer_network_public_key_hex",
            request.peer_network_public_key_hex,
        )?;
        validate_key_hex(
            "peer_ephemeral_public_key_hex",
            request.peer_ephemeral_public_key_hex,
        )?;

        let tx = self.conn.transaction()?;
        let fabric_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM fabric_roots WHERE fabric_id = ?1)",
            [request.fabric_id],
            |row| row.get(0),
        )?;
        if !fabric_exists {
            return Err(crate::DevRelayError::Config(format!(
                "unknown fabric {}",
                request.fabric_id
            )));
        }

        let ephemeral = generate_ephemeral_pairing_key()?;
        let transcript_hash_hex = compute_handshake_transcript_hash(
            request.fabric_id,
            request.local_device_id,
            request.peer_device_id,
            &ephemeral.public_key_hex,
            request.peer_ephemeral_public_key_hex,
            request.anchor_address,
        )?;
        let short_authentication_string = derive_short_authentication_string(&transcript_hash_hex)?;
        let now = unix_now_seconds();
        let session = PairingSession {
            pairing_id: generate_pairing_id(),
            fabric_id: request.fabric_id.to_string(),
            local_device_id: request.local_device_id.to_string(),
            peer_device_id: request.peer_device_id.to_string(),
            peer_display_name: request.peer_display_name.to_string(),
            peer_signing_public_key_hex: request.peer_signing_public_key_hex.to_string(),
            peer_network_public_key_hex: request.peer_network_public_key_hex.to_string(),
            anchor_address: request.anchor_address.map(ToString::to_string),
            local_ephemeral_public_key_hex: ephemeral.public_key_hex,
            peer_ephemeral_public_key_hex: request.peer_ephemeral_public_key_hex.to_string(),
            transcript_hash_hex,
            short_authentication_string,
            state: PairingState::Pending,
            certificate_json: None,
            expires_at_unix_seconds: now.saturating_add(request.ttl_seconds.max(1)),
            confirmed_at_unix_seconds: None,
            aborted_at_unix_seconds: None,
            created_at_unix_seconds: now,
        };
        tx.execute(
            r#"
INSERT INTO pairing_sessions (
    pairing_id,
    fabric_id,
    local_device_id,
    peer_device_id,
    peer_display_name,
    peer_signing_public_key_hex,
    peer_network_public_key_hex,
    anchor_address,
    local_ephemeral_public_key_hex,
    peer_ephemeral_public_key_hex,
    transcript_hash_hex,
    short_authentication_string,
    state,
    certificate_json,
    expires_at_unix_seconds,
    confirmed_at_unix_seconds,
    aborted_at_unix_seconds,
    created_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
"#,
            rusqlite::params![
                session.pairing_id.as_str(),
                session.fabric_id.as_str(),
                session.local_device_id.as_str(),
                session.peer_device_id.as_str(),
                session.peer_display_name.as_str(),
                session.peer_signing_public_key_hex.as_str(),
                session.peer_network_public_key_hex.as_str(),
                session.anchor_address.as_deref(),
                session.local_ephemeral_public_key_hex.as_str(),
                session.peer_ephemeral_public_key_hex.as_str(),
                session.transcript_hash_hex.as_str(),
                session.short_authentication_string.as_str(),
                session.state.as_str(),
                session.certificate_json.as_deref(),
                session.expires_at_unix_seconds as i64,
                session.confirmed_at_unix_seconds.map(|value| value as i64),
                session.aborted_at_unix_seconds.map(|value| value as i64),
                session.created_at_unix_seconds as i64,
            ],
        )?;
        tx.commit()?;
        Ok(session)
    }

    pub fn get_pairing_session(&self, pairing_id: &str) -> Result<Option<PairingSession>> {
        self.conn
            .query_row(
                pairing_session_select_sql("WHERE pairing_id = ?1").as_str(),
                [pairing_id],
                pairing_session_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn confirm_pairing_session(
        &mut self,
        pairing_id: &str,
        observed_code: &str,
        certificate_json: &str,
        now_unix_seconds: u64,
    ) -> Result<PairingSession> {
        serde_json::from_str::<serde_json::Value>(certificate_json)?;
        let tx = self.conn.transaction()?;
        let session = tx
            .query_row(
                pairing_session_select_sql("WHERE pairing_id = ?1").as_str(),
                [pairing_id],
                pairing_session_from_row,
            )
            .optional()?
            .ok_or_else(|| {
                crate::DevRelayError::Config(format!("unknown pairing session {pairing_id}"))
            })?;
        if session.state.is_terminal() {
            return Err(crate::DevRelayError::Config(format!(
                "pairing session is already {}",
                session.state.as_str()
            )));
        }
        if now_unix_seconds > session.expires_at_unix_seconds {
            tx.execute(
                "UPDATE pairing_sessions SET state = ?1 WHERE pairing_id = ?2",
                (PairingState::Expired.as_str(), pairing_id),
            )?;
            tx.commit()?;
            return Err(crate::DevRelayError::Config(
                "pairing session expired".to_string(),
            ));
        }
        if observed_code != session.short_authentication_string {
            return Err(crate::DevRelayError::Config(
                "pairing code mismatch".to_string(),
            ));
        }

        tx.execute(
            r#"
UPDATE pairing_sessions
SET state = ?1,
    certificate_json = ?2,
    confirmed_at_unix_seconds = ?3
WHERE pairing_id = ?4 AND state = ?5
"#,
            (
                PairingState::Confirmed.as_str(),
                certificate_json,
                now_unix_seconds as i64,
                pairing_id,
                PairingState::Pending.as_str(),
            ),
        )?;
        tx.execute(
            r#"
INSERT INTO device_public_identities (
    device_id,
    fabric_id,
    display_name,
    signing_public_key_hex,
    network_public_key_hex,
    platform_key,
    architecture,
    created_at_unix_seconds,
    last_seen_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT(device_id) DO UPDATE SET
    fabric_id = excluded.fabric_id,
    display_name = excluded.display_name,
    signing_public_key_hex = excluded.signing_public_key_hex,
    network_public_key_hex = excluded.network_public_key_hex,
    last_seen_unix_seconds = excluded.last_seen_unix_seconds
"#,
            (
                session.peer_device_id.as_str(),
                session.fabric_id.as_str(),
                session.peer_display_name.as_str(),
                session.peer_signing_public_key_hex.as_str(),
                session.peer_network_public_key_hex.as_str(),
                "unknown",
                "unknown",
                session.created_at_unix_seconds as i64,
                now_unix_seconds as i64,
            ),
        )?;
        let mut audit = AuditEventInput::new(
            AuditEventType::DevicePaired,
            AuditOutcome::Succeeded,
            format!("paired device {}", session.peer_display_name),
        )
        .with_detail(serde_json::json!({
            "pairing_id": session.pairing_id,
            "fabric_id": session.fabric_id,
            "confirmed_at_unix_seconds": now_unix_seconds,
        }));
        audit.actor_device_id = Some(session.local_device_id);
        audit.target_device_id = Some(session.peer_device_id);
        insert_audit_event_tx(&tx, audit, now_unix_seconds)?;
        tx.commit()?;
        self.get_pairing_session(pairing_id)?.ok_or_else(|| {
            crate::DevRelayError::Config("pairing confirmation disappeared".to_string())
        })
    }

    pub fn abort_pairing_session(
        &mut self,
        pairing_id: &str,
        now_unix_seconds: u64,
    ) -> Result<PairingSession> {
        let tx = self.conn.transaction()?;
        let session = tx
            .query_row(
                pairing_session_select_sql("WHERE pairing_id = ?1").as_str(),
                [pairing_id],
                pairing_session_from_row,
            )
            .optional()?
            .ok_or_else(|| {
                crate::DevRelayError::Config(format!("unknown pairing session {pairing_id}"))
            })?;
        if session.state.is_terminal() {
            return Err(crate::DevRelayError::Config(format!(
                "pairing session is already {}",
                session.state.as_str()
            )));
        }
        tx.execute(
            "UPDATE pairing_sessions SET state = ?1, aborted_at_unix_seconds = ?2 WHERE pairing_id = ?3",
            (
                PairingState::Aborted.as_str(),
                now_unix_seconds as i64,
                pairing_id,
            ),
        )?;
        tx.commit()?;
        self.get_pairing_session(pairing_id)?
            .ok_or_else(|| crate::DevRelayError::Config("pairing abort disappeared".to_string()))
    }

    pub fn expire_pairing_sessions(
        &mut self,
        now_unix_seconds: u64,
    ) -> Result<Vec<PairingSession>> {
        let ids = {
            let mut statement = self.conn.prepare(
                r#"
SELECT pairing_id
FROM pairing_sessions
WHERE state = ?1 AND expires_at_unix_seconds < ?2
ORDER BY expires_at_unix_seconds ASC, pairing_id ASC
"#,
            )?;
            let rows = statement.query_map(
                (PairingState::Pending.as_str(), now_unix_seconds as i64),
                |row| row.get::<_, String>(0),
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let tx = self.conn.transaction()?;
        for id in &ids {
            tx.execute(
                "UPDATE pairing_sessions SET state = ?1 WHERE pairing_id = ?2",
                (PairingState::Expired.as_str(), id.as_str()),
            )?;
        }
        tx.commit()?;
        ids.into_iter()
            .map(|id| {
                self.get_pairing_session(&id)?.ok_or_else(|| {
                    crate::DevRelayError::Config("expired pairing disappeared".to_string())
                })
            })
            .collect()
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

    pub fn list_leases(&self, project_id: Option<&str>) -> Result<Vec<LeaseRecord>> {
        let sql = r#"
SELECT lease_id,
       project_id,
       session_id,
       state,
       epoch,
       holder_device_id,
       latest_snapshot_id,
       handoff_id
FROM leases
"#;
        if let Some(project_id) = project_id {
            let mut statement = self.conn.prepare(&format!(
                "{sql} WHERE project_id = ?1 ORDER BY project_id ASC, session_id ASC, lease_id ASC"
            ))?;
            let rows = statement.query_map([project_id], lease_record_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        } else {
            let mut statement = self.conn.prepare(&format!(
                "{sql} ORDER BY project_id ASC, session_id ASC, lease_id ASC"
            ))?;
            let rows = statement.query_map([], lease_record_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        }
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
        ensure_device_not_revoked_conn(&tx, request.holder_device_id, "snapshot publish")?;
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
        let mut audit = AuditEventInput::new(
            AuditEventType::SnapshotPublished,
            if stale_error.is_some() {
                AuditOutcome::Blocked
            } else {
                AuditOutcome::Succeeded
            },
            if stale_error.is_some() {
                "snapshot stored without advancing canonical latest"
            } else {
                "snapshot published as canonical latest"
            },
        )
        .with_detail(serde_json::json!({
            "expected_epoch": request.expected_epoch,
            "actual_epoch": lease.epoch,
            "expected_latest_snapshot_id": request.expected_latest_snapshot_id,
            "previous_latest_snapshot_id": lease.latest_snapshot_id,
            "pinned": request.pinned,
            "label": request.label,
        }));
        audit.project_id = Some(request.metadata.project_id.clone());
        audit.actor_device_id = Some(request.holder_device_id.to_string());
        audit.session_id = Some(request.session_id.to_string());
        audit.snapshot_id = Some(request.metadata.snapshot_id.clone());
        audit.lease_id = Some(request.lease_id.to_string());
        insert_audit_event_tx(&tx, audit, request.metadata.created_at_unix_seconds)?;
        tx.commit()?;

        if let Some(err) = stale_error {
            return Err(err);
        }

        Ok(CanonicalPublishResult {
            snapshot,
            latest_snapshot_id: request.metadata.snapshot_id.clone(),
        })
    }

    pub fn publish_inactive_snapshot_as_fork(
        &mut self,
        request: InactiveForkPublishRequest<'_>,
    ) -> Result<InactiveForkPublishResult> {
        if request.metadata.session_id.as_deref() != Some(request.session_id) {
            return Err(crate::DevRelayError::Config(
                "snapshot session_id does not match inactive publish session".to_string(),
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
                "lease session_id does not match inactive publish session".to_string(),
            ));
        }
        if lease.holder_device_id.as_deref() != Some(request.holder_device_id) {
            return Err(crate::DevRelayError::Config(
                "inactive publish rejected: holder device mismatch".to_string(),
            ));
        }
        ensure_device_not_revoked_conn(&tx, request.holder_device_id, "inactive publish")?;
        if lease.state != LeaseState::Inactive {
            return Err(crate::DevRelayError::Config(format!(
                "inactive edit fork requires inactive lease; found {}",
                lease.state.as_str()
            )));
        }

        let parent_session = tx
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
WHERE session_id = ?1 AND project_id = ?2
"#,
                (request.session_id, request.metadata.project_id.as_str()),
                stored_session_from_row,
            )
            .optional()?
            .ok_or_else(|| {
                crate::DevRelayError::Config(format!("unknown session {}", request.session_id))
            })?;

        let mut forked_lease = lease.clone();
        forked_lease.state = LeaseState::Forked;
        lease.validate_transition_to(&forked_lease)?;

        let now = unix_now_seconds();
        let fork_session_id = generate_session_id();
        let fork_name = format!("Separate work from {}", parent_session.name);
        tx.execute(
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
                fork_session_id.as_str(),
                parent_session.project_id.as_str(),
                parent_session.active_workspace_id.as_deref(),
                SessionState::Fork.as_str(),
                fork_name.as_str(),
                parent_session.session_id.as_str(),
                Option::<i64>::None,
                now as i64,
                now as i64,
            ),
        )?;
        let fork_session = StoredSession {
            session_id: fork_session_id.clone(),
            project_id: parent_session.project_id.clone(),
            name: fork_name,
            parent_session_id: Some(parent_session.session_id.clone()),
            active_workspace_id: parent_session.active_workspace_id.clone(),
            state: SessionState::Fork,
            archived_at_unix_seconds: None,
            created_at_unix_seconds: now,
            updated_at_unix_seconds: now,
        };

        let mut fork_metadata = request.metadata.clone();
        fork_metadata.session_id = Some(fork_session_id.clone());
        if fork_metadata.parent_snapshot_id.is_none() {
            fork_metadata.parent_snapshot_id = lease.latest_snapshot_id.clone();
        }
        fork_metadata.validate()?;

        let metadata_json = serde_json::to_string(&fork_metadata)?;
        let sequence_number: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence_number), 0) + 1 FROM snapshots WHERE project_id = ?1",
            [fork_metadata.project_id.as_str()],
            |row| row.get(0),
        )?;
        let label = request
            .label
            .map(ToString::to_string)
            .unwrap_or_else(|| "separate work from inactive workspace".to_string());
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
                fork_metadata.snapshot_id.as_str(),
                fork_metadata.project_id.as_str(),
                fork_session_id.as_str(),
                fork_metadata.parent_snapshot_id.as_deref(),
                sequence_number,
                true,
                label.as_str(),
                metadata_json.as_str(),
                fork_metadata.created_at_unix_seconds as i64,
            ),
        )?;
        let changed = tx.execute(
            "UPDATE leases SET state = ?1 WHERE lease_id = ?2 AND state = ?3",
            (
                LeaseState::Forked.as_str(),
                request.lease_id,
                LeaseState::Inactive.as_str(),
            ),
        )?;
        if changed != 1 {
            return Err(crate::DevRelayError::Config(
                "inactive edit fork lost lease state race".to_string(),
            ));
        }
        let mut audit = AuditEventInput::new(
            AuditEventType::SnapshotPublished,
            AuditOutcome::Blocked,
            "inactive workspace snapshot stored as fork",
        )
        .with_detail(serde_json::json!({
            "source_session_id": request.session_id,
            "fork_session_id": fork_session_id.as_str(),
            "canonical_latest_snapshot_id": lease.latest_snapshot_id.as_deref(),
            "pinned": true,
            "label": label.as_str(),
        }));
        audit.project_id = Some(fork_metadata.project_id.clone());
        audit.actor_device_id = Some(request.holder_device_id.to_string());
        audit.session_id = Some(fork_session_id.clone());
        audit.snapshot_id = Some(fork_metadata.snapshot_id.clone());
        audit.lease_id = Some(request.lease_id.to_string());
        insert_audit_event_tx(&tx, audit, fork_metadata.created_at_unix_seconds)?;
        tx.commit()?;

        Ok(InactiveForkPublishResult {
            snapshot: StoredSnapshot {
                snapshot_id: fork_metadata.snapshot_id.clone(),
                project_id: fork_metadata.project_id.clone(),
                session_id: Some(fork_session_id),
                parent_snapshot_id: fork_metadata.parent_snapshot_id.clone(),
                sequence_number,
                pinned: true,
                label: Some(label),
                metadata: fork_metadata,
                created_at_unix_seconds: request.metadata.created_at_unix_seconds,
            },
            fork_session,
            canonical_latest_snapshot_id: lease.latest_snapshot_id,
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
        ensure_device_not_revoked_conn(&tx, source_device_id, "begin handoff")?;
        ensure_device_not_revoked_conn(&tx, target_device_id, "begin handoff")?;

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
        insert_handoff_journal(&tx, &handoff, HandoffJournalPhase::Begin, "{}", now)?;
        insert_handoff_journal(&tx, &handoff, HandoffJournalPhase::TargetPrepare, "{}", now)?;
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

    pub fn record_handoff_target_apply(
        &mut self,
        handoff_id: &str,
        detail_json: Option<&str>,
    ) -> Result<HandoffJournalRecord> {
        let detail_json = normalize_journal_detail(detail_json)?;
        let tx = self.conn.transaction()?;
        let handoff = tx
            .query_row(
                handoff_select_sql("WHERE handoff_id = ?1").as_str(),
                [handoff_id],
                handoff_record_from_row,
            )
            .optional()?
            .ok_or_else(|| crate::DevRelayError::Config(format!("unknown handoff {handoff_id}")))?;
        if handoff.state.is_terminal() {
            return Err(crate::DevRelayError::Config(format!(
                "cannot record target apply for {} handoff",
                handoff.state.as_str()
            )));
        }
        let record = insert_handoff_journal(
            &tx,
            &handoff,
            HandoffJournalPhase::TargetApply,
            &detail_json,
            unix_now_seconds(),
        )?;
        tx.commit()?;
        Ok(record)
    }

    pub fn mark_handoff_target_verified(&mut self, handoff_id: &str) -> Result<HandoffRecord> {
        self.update_handoff_state(
            handoff_id,
            HandoffState::TargetPrepare,
            HandoffState::TargetVerified,
            HandoffJournalPhase::TargetVerified,
        )
    }

    pub fn mark_handoff_source_ready(&mut self, handoff_id: &str) -> Result<HandoffRecord> {
        self.update_handoff_state(
            handoff_id,
            HandoffState::TargetVerified,
            HandoffState::SourceReady,
            HandoffJournalPhase::SourceReady,
        )
    }

    pub fn abort_handoff(&mut self, handoff_id: &str) -> Result<HandoffRecord> {
        self.abort_handoff_at(handoff_id, unix_now_seconds(), "manual abort")
    }

    pub fn commit_handoff(
        &mut self,
        handoff_id: &str,
        observed_source_generation: &str,
        now_unix_seconds: u64,
    ) -> Result<HandoffRecord> {
        self.commit_handoff_inner(
            handoff_id,
            observed_source_generation,
            now_unix_seconds,
            None,
        )
    }

    pub fn commit_handoff_with_snapshot_preflight(
        &mut self,
        handoff_id: &str,
        observed_source_generation: &str,
        now_unix_seconds: u64,
        preflight: HandoffCommitSnapshotPreflight<'_>,
    ) -> Result<HandoffRecord> {
        self.commit_handoff_inner(
            handoff_id,
            observed_source_generation,
            now_unix_seconds,
            Some(preflight),
        )
    }

    fn commit_handoff_inner(
        &mut self,
        handoff_id: &str,
        observed_source_generation: &str,
        now_unix_seconds: u64,
        preflight: Option<HandoffCommitSnapshotPreflight<'_>>,
    ) -> Result<HandoffRecord> {
        let fault = self.fault;
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
        ensure_device_not_revoked_conn(&tx, &handoff.target_device_id, "handoff commit")?;
        if let Some(preflight) = preflight {
            preflight.verify(&handoff)?;
        }

        let changed = tx.execute(
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
        if changed != 1 {
            return Err(crate::DevRelayError::Config(
                "handoff commit lost lease state race".to_string(),
            ));
        }
        inject_metadata_db_fault(fault, MetadataDbFaultPoint::DuringLeaseCommit)?;
        tx.execute(
            "UPDATE handoffs SET state = ?1, committed_at_unix_seconds = ?2 WHERE handoff_id = ?3",
            (
                HandoffState::Committed.as_str(),
                now_unix_seconds as i64,
                handoff_id,
            ),
        )?;
        let committed = HandoffRecord {
            state: HandoffState::Committed,
            ..handoff.clone()
        };
        insert_handoff_journal(
            &tx,
            &committed,
            HandoffJournalPhase::LeaseCommitted,
            "{}",
            now_unix_seconds,
        )?;
        let mut audit = AuditEventInput::new(
            AuditEventType::LeaseTransferred,
            AuditOutcome::Succeeded,
            "lease transferred to target device",
        )
        .with_detail(serde_json::json!({
            "from_device_id": handoff.source_device_id.as_str(),
            "to_device_id": handoff.target_device_id.as_str(),
            "previous_epoch": handoff.expected_epoch,
            "next_epoch": handoff.expected_epoch.saturating_add(1),
        }));
        audit.project_id = Some(handoff.project_id.clone());
        audit.actor_device_id = Some(handoff.source_device_id.clone());
        audit.target_device_id = Some(handoff.target_device_id.clone());
        audit.lease_id = Some(handoff.lease_id.clone());
        audit.handoff_id = Some(handoff_id.to_string());
        insert_audit_event_tx(&tx, audit, now_unix_seconds)?;
        tx.commit()?;
        self.get_handoff(handoff_id)?
            .ok_or_else(|| crate::DevRelayError::Config("handoff commit disappeared".to_string()))
    }

    pub fn recover_handoff(
        &mut self,
        handoff_id: &str,
        observed_source_generation: &str,
        now_unix_seconds: u64,
    ) -> Result<HandoffRecoveryOutcome> {
        let handoff = self
            .get_handoff(handoff_id)?
            .ok_or_else(|| crate::DevRelayError::Config(format!("unknown handoff {handoff_id}")))?;
        match handoff.state {
            HandoffState::Committed => return Ok(HandoffRecoveryOutcome::AlreadyCommitted),
            HandoffState::Aborted => return Ok(HandoffRecoveryOutcome::AlreadyAborted),
            _ => {}
        }
        if now_unix_seconds > handoff.expires_at_unix_seconds {
            self.abort_handoff_at(handoff_id, now_unix_seconds, "expired during recovery")?;
            return Ok(HandoffRecoveryOutcome::AbortedExpired);
        }
        if handoff.state == HandoffState::SourceReady {
            self.commit_handoff(handoff_id, observed_source_generation, now_unix_seconds)?;
            return Ok(HandoffRecoveryOutcome::Committed);
        }
        Ok(HandoffRecoveryOutcome::WaitingForTarget)
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

    pub fn list_handoffs(&self, project_id: Option<&str>) -> Result<Vec<HandoffRecord>> {
        if let Some(project_id) = project_id {
            let mut statement = self.conn.prepare(
                handoff_select_sql("WHERE project_id = ?1 ORDER BY project_id ASC, handoff_id ASC")
                    .as_str(),
            )?;
            let rows = statement.query_map([project_id], handoff_record_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        } else {
            let mut statement = self
                .conn
                .prepare(handoff_select_sql("ORDER BY project_id ASC, handoff_id ASC").as_str())?;
            let rows = statement.query_map([], handoff_record_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        }
    }

    pub fn list_handoff_journal(&self, handoff_id: &str) -> Result<Vec<HandoffJournalRecord>> {
        let mut statement = self.conn.prepare(
            r#"
SELECT journal_id,
       handoff_id,
       lease_id,
       project_id,
       phase,
       detail_json,
       created_at_unix_seconds
FROM handoff_journal
WHERE handoff_id = ?1
ORDER BY journal_id ASC
"#,
        )?;
        let rows = statement.query_map([handoff_id], handoff_journal_record_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn abort_handoff_at(
        &mut self,
        handoff_id: &str,
        now_unix_seconds: u64,
        reason: &str,
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
        if handoff.state == HandoffState::Committed {
            return Err(crate::DevRelayError::Config(
                "cannot abort committed handoff".to_string(),
            ));
        }
        if handoff.state == HandoffState::Aborted {
            return Ok(handoff);
        }

        tx.execute(
            "UPDATE handoffs SET state = ?1, aborted_at_unix_seconds = ?2 WHERE handoff_id = ?3",
            (
                HandoffState::Aborted.as_str(),
                now_unix_seconds as i64,
                handoff_id,
            ),
        )?;
        tx.execute(
            "UPDATE leases SET state = ?1, handoff_id = NULL WHERE lease_id = ?2 AND handoff_id = ?3",
            (LeaseState::Active.as_str(), handoff.lease_id.as_str(), handoff_id),
        )?;
        let aborted = HandoffRecord {
            state: HandoffState::Aborted,
            ..handoff
        };
        let detail = serde_json::json!({
            "reason": reason
        })
        .to_string();
        let detail_json = normalize_journal_detail(Some(&detail))?;
        insert_handoff_journal(
            &tx,
            &aborted,
            HandoffJournalPhase::Aborted,
            &detail_json,
            now_unix_seconds,
        )?;
        tx.commit()?;
        Ok(aborted)
    }

    fn update_handoff_state(
        &mut self,
        handoff_id: &str,
        expected: HandoffState,
        next: HandoffState,
        journal_phase: HandoffJournalPhase,
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
        if handoff.state != expected {
            return Err(crate::DevRelayError::Config(format!(
                "handoff {handoff_id} is not {}",
                expected.as_str()
            )));
        }
        let changed = tx.execute(
            "UPDATE handoffs SET state = ?1 WHERE handoff_id = ?2 AND state = ?3",
            (next.as_str(), handoff_id, expected.as_str()),
        )?;
        if changed == 0 {
            return Err(crate::DevRelayError::Config(format!(
                "handoff {handoff_id} is not {}",
                expected.as_str()
            )));
        }
        let updated = HandoffRecord {
            state: next,
            ..handoff
        };
        insert_handoff_journal(&tx, &updated, journal_phase, "{}", unix_now_seconds())?;
        tx.commit()?;
        Ok(updated)
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

fn fabric_root_identity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FabricRootIdentity> {
    Ok(FabricRootIdentity {
        fabric_id: row.get(0)?,
        fabric_name: row.get(1)?,
        root_public_key_hex: row.get(2)?,
        created_at_unix_seconds: row.get::<_, i64>(3)?.max(0) as u64,
        rotation_epoch: row.get::<_, i64>(4)?.max(0) as u64,
    })
}

fn device_public_identity_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<DevicePublicIdentity> {
    Ok(DevicePublicIdentity {
        device_id: row.get(0)?,
        display_name: row.get(1)?,
        fabric_id: row.get(2)?,
        signing_public_key_hex: row.get(3)?,
        network_public_key_hex: row.get(4)?,
        platform_key: row.get(5)?,
        architecture: row.get(6)?,
        created_at_unix_seconds: row.get::<_, i64>(7)?.max(0) as u64,
        last_seen_unix_seconds: row.get::<_, i64>(8)?.max(0) as u64,
    })
}

fn pairing_session_select_sql(where_clause: &str) -> String {
    format!(
        r#"
SELECT pairing_id,
       fabric_id,
       local_device_id,
       peer_device_id,
       peer_display_name,
       peer_signing_public_key_hex,
       peer_network_public_key_hex,
       anchor_address,
       local_ephemeral_public_key_hex,
       peer_ephemeral_public_key_hex,
       transcript_hash_hex,
       short_authentication_string,
       state,
       certificate_json,
       expires_at_unix_seconds,
       confirmed_at_unix_seconds,
       aborted_at_unix_seconds,
       created_at_unix_seconds
FROM pairing_sessions
{where_clause}
"#
    )
}

fn pairing_session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PairingSession> {
    Ok(PairingSession {
        pairing_id: row.get(0)?,
        fabric_id: row.get(1)?,
        local_device_id: row.get(2)?,
        peer_device_id: row.get(3)?,
        peer_display_name: row.get(4)?,
        peer_signing_public_key_hex: row.get(5)?,
        peer_network_public_key_hex: row.get(6)?,
        anchor_address: row.get(7)?,
        local_ephemeral_public_key_hex: row.get(8)?,
        peer_ephemeral_public_key_hex: row.get(9)?,
        transcript_hash_hex: row.get(10)?,
        short_authentication_string: row.get(11)?,
        state: PairingState::parse(&row.get::<_, String>(12)?),
        certificate_json: row.get(13)?,
        expires_at_unix_seconds: row.get::<_, i64>(14)?.max(0) as u64,
        confirmed_at_unix_seconds: row
            .get::<_, Option<i64>>(15)?
            .map(|value| value.max(0) as u64),
        aborted_at_unix_seconds: row
            .get::<_, Option<i64>>(16)?
            .map(|value| value.max(0) as u64),
        created_at_unix_seconds: row.get::<_, i64>(17)?.max(0) as u64,
    })
}

fn validate_audit_input(input: &AuditEventInput) -> Result<()> {
    if input.summary.trim().is_empty() {
        return Err(crate::DevRelayError::Config(
            "audit summary must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn normalize_audit_detail(detail: &serde_json::Value) -> Result<String> {
    Ok(serde_json::to_string(detail)?)
}

fn insert_audit_event_tx(
    tx: &Transaction<'_>,
    input: AuditEventInput,
    created_at_unix_seconds: u64,
) -> Result<AuditEventRecord> {
    validate_audit_input(&input)?;
    let detail_json = normalize_audit_detail(&input.detail)?;
    tx.execute(
        r#"
INSERT INTO audit_events (
    event_type,
    project_id,
    actor_device_id,
    target_device_id,
    session_id,
    snapshot_id,
    lease_id,
    handoff_id,
    outcome,
    summary,
    detail_json,
    created_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
"#,
        (
            input.event_type.as_str(),
            input.project_id.as_deref(),
            input.actor_device_id.as_deref(),
            input.target_device_id.as_deref(),
            input.session_id.as_deref(),
            input.snapshot_id.as_deref(),
            input.lease_id.as_deref(),
            input.handoff_id.as_deref(),
            input.outcome.as_str(),
            input.summary.as_str(),
            detail_json.as_str(),
            created_at_unix_seconds as i64,
        ),
    )?;
    Ok(AuditEventRecord {
        schema_version: AUDIT_SCHEMA_VERSION,
        audit_id: tx.last_insert_rowid(),
        event_type: input.event_type,
        project_id: input.project_id,
        actor_device_id: input.actor_device_id,
        target_device_id: input.target_device_id,
        session_id: input.session_id,
        snapshot_id: input.snapshot_id,
        lease_id: input.lease_id,
        handoff_id: input.handoff_id,
        outcome: input.outcome,
        summary: input.summary,
        detail: serde_json::from_str(&detail_json)?,
        created_at_unix_seconds,
    })
}

fn audit_event_record_from_row(row: &Row<'_>) -> rusqlite::Result<AuditEventRecord> {
    let detail_json: String = row.get(11)?;
    let detail = serde_json::from_str(&detail_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, Box::new(err))
    })?;
    Ok(AuditEventRecord {
        schema_version: AUDIT_SCHEMA_VERSION,
        audit_id: row.get(0)?,
        event_type: AuditEventType::parse(&row.get::<_, String>(1)?),
        project_id: row.get(2)?,
        actor_device_id: row.get(3)?,
        target_device_id: row.get(4)?,
        session_id: row.get(5)?,
        snapshot_id: row.get(6)?,
        lease_id: row.get(7)?,
        handoff_id: row.get(8)?,
        outcome: AuditOutcome::parse(&row.get::<_, String>(9)?),
        summary: row.get(10)?,
        detail,
        created_at_unix_seconds: row.get::<_, i64>(12)?.max(0) as u64,
    })
}

fn device_revocation_from_row(row: &Row<'_>) -> rusqlite::Result<DeviceRevocationRecord> {
    Ok(DeviceRevocationRecord {
        device_id: row.get(0)?,
        revoked_by_device_id: row.get(1)?,
        reason: row.get(2)?,
        key_rotation_required: row.get(3)?,
        revoked_at_unix_seconds: row.get::<_, i64>(4)?.max(0) as u64,
    })
}

fn ensure_device_not_revoked_conn(
    conn: &Connection,
    device_id: &str,
    operation: &str,
) -> Result<()> {
    if let Some(revocation) = conn
        .query_row(
            r#"
SELECT device_id,
       revoked_by_device_id,
       reason,
       key_rotation_required,
       revoked_at_unix_seconds
FROM device_revocations
WHERE device_id = ?1
"#,
            [device_id],
            device_revocation_from_row,
        )
        .optional()?
    {
        return Err(crate::DevRelayError::Config(format!(
            "{operation} rejected: device {} was revoked by {} at {}: {}",
            revocation.device_id,
            revocation.revoked_by_device_id,
            revocation.revoked_at_unix_seconds,
            revocation.reason
        )));
    }
    Ok(())
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(crate::DevRelayError::Config(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
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

fn insert_handoff_journal(
    tx: &Transaction<'_>,
    handoff: &HandoffRecord,
    phase: HandoffJournalPhase,
    detail_json: &str,
    created_at_unix_seconds: u64,
) -> Result<HandoffJournalRecord> {
    tx.execute(
        r#"
INSERT INTO handoff_journal (
    handoff_id,
    lease_id,
    project_id,
    phase,
    detail_json,
    created_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
"#,
        (
            handoff.handoff_id.as_str(),
            handoff.lease_id.as_str(),
            handoff.project_id.as_str(),
            phase.as_str(),
            detail_json,
            created_at_unix_seconds as i64,
        ),
    )?;
    Ok(HandoffJournalRecord {
        journal_id: tx.last_insert_rowid(),
        handoff_id: handoff.handoff_id.clone(),
        lease_id: handoff.lease_id.clone(),
        project_id: handoff.project_id.clone(),
        phase,
        detail_json: detail_json.to_string(),
        created_at_unix_seconds,
    })
}

fn handoff_journal_record_from_row(row: &Row<'_>) -> rusqlite::Result<HandoffJournalRecord> {
    Ok(HandoffJournalRecord {
        journal_id: row.get(0)?,
        handoff_id: row.get(1)?,
        lease_id: row.get(2)?,
        project_id: row.get(3)?,
        phase: HandoffJournalPhase::parse(&row.get::<_, String>(4)?),
        detail_json: row.get(5)?,
        created_at_unix_seconds: row.get::<_, i64>(6)?.max(0) as u64,
    })
}

fn inject_metadata_db_fault(
    configured: Option<MetadataDbFaultPoint>,
    fault: MetadataDbFaultPoint,
) -> Result<()> {
    if configured == Some(fault) {
        return Err(crate::DevRelayError::Config(format!(
            "injected metadata DB fault at {}",
            fault.as_str()
        )));
    }
    Ok(())
}

fn normalize_journal_detail(detail_json: Option<&str>) -> Result<String> {
    let value = match detail_json {
        Some(detail_json) => serde_json::from_str::<serde_json::Value>(detail_json)?,
        None => serde_json::json!({}),
    };
    Ok(serde_json::to_string(&value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CAS_HASH_PREFIX, CasChunkHash, DEFAULT_SIDECAR_CHUNK_BYTES, LocalConfig, SnapshotMetadata,
        SnapshotSidecar, classification_reason,
    };

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
            "handoff_journal",
            "fabric_roots",
            "device_public_identities",
            "pairing_sessions",
            "audit_events",
            "device_revocations",
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
        assert!(index_exists(&db, "idx_handoff_journal_handoff"));
        assert!(index_exists(&db, "idx_handoff_journal_project_phase"));
        assert!(index_exists(&db, "idx_device_public_identities_fabric"));
        assert!(index_exists(&db, "idx_pairing_sessions_fabric_state"));
        assert!(index_exists(&db, "idx_pairing_sessions_peer"));
        assert!(index_exists(&db, "idx_audit_events_timeline"));
        assert!(index_exists(&db, "idx_audit_events_project_timeline"));
        assert!(index_exists(&db, "idx_audit_events_type_timeline"));
        assert!(index_exists(&db, "idx_audit_events_snapshot"));
        assert!(index_exists(&db, "idx_audit_events_lease"));
        assert!(index_exists(&db, "idx_device_revocations_revoked_at"));

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
        for column in [
            "journal_id",
            "handoff_id",
            "lease_id",
            "project_id",
            "phase",
            "detail_json",
            "created_at_unix_seconds",
        ] {
            assert!(column_exists(&db, "handoff_journal", column), "{column}");
        }
        for column in [
            "fabric_id",
            "fabric_name",
            "root_public_key_hex",
            "created_at_unix_seconds",
            "rotation_epoch",
        ] {
            assert!(column_exists(&db, "fabric_roots", column), "{column}");
        }
        for column in [
            "device_id",
            "fabric_id",
            "display_name",
            "signing_public_key_hex",
            "network_public_key_hex",
            "platform_key",
            "architecture",
            "created_at_unix_seconds",
            "last_seen_unix_seconds",
        ] {
            assert!(
                column_exists(&db, "device_public_identities", column),
                "{column}"
            );
        }
        for column in [
            "pairing_id",
            "fabric_id",
            "local_device_id",
            "peer_device_id",
            "peer_display_name",
            "peer_signing_public_key_hex",
            "peer_network_public_key_hex",
            "anchor_address",
            "local_ephemeral_public_key_hex",
            "peer_ephemeral_public_key_hex",
            "transcript_hash_hex",
            "short_authentication_string",
            "state",
            "certificate_json",
            "expires_at_unix_seconds",
            "confirmed_at_unix_seconds",
            "aborted_at_unix_seconds",
            "created_at_unix_seconds",
        ] {
            assert!(column_exists(&db, "pairing_sessions", column), "{column}");
        }
        assert!(column_exists(&db, "task_runs", "task_run_id"));
        assert!(column_exists(&db, "task_runs", "metadata_json"));
        for column in [
            "audit_id",
            "event_type",
            "project_id",
            "actor_device_id",
            "target_device_id",
            "session_id",
            "snapshot_id",
            "lease_id",
            "handoff_id",
            "outcome",
            "summary",
            "detail_json",
            "created_at_unix_seconds",
        ] {
            assert!(column_exists(&db, "audit_events", column), "{column}");
        }
        for column in [
            "device_id",
            "revoked_by_device_id",
            "reason",
            "key_rotation_required",
            "revoked_at_unix_seconds",
        ] {
            assert!(column_exists(&db, "device_revocations", column), "{column}");
        }
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
            "handoff_journal",
            "handoff_id",
            "handoffs"
        ));
        assert!(foreign_key_exists(
            &db,
            "handoff_journal",
            "project_id",
            "projects"
        ));
        assert!(foreign_key_exists(
            &db,
            "device_public_identities",
            "fabric_id",
            "fabric_roots"
        ));
        assert!(foreign_key_exists(
            &db,
            "pairing_sessions",
            "fabric_id",
            "fabric_roots"
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
    fn stores_public_fabric_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();
        let root = FabricRootIdentity {
            fabric_id: "f_1234567890abcdef12345678".to_string(),
            fabric_name: "Test Fabric".to_string(),
            root_public_key_hex: "a".repeat(64),
            created_at_unix_seconds: 100,
            rotation_epoch: 0,
        };
        let device = DevicePublicIdentity {
            device_id: "d_1234567890abcdef12345678".to_string(),
            display_name: "Laptop".to_string(),
            fabric_id: root.fabric_id.clone(),
            signing_public_key_hex: "b".repeat(64),
            network_public_key_hex: "c".repeat(64),
            platform_key: "macos".to_string(),
            architecture: "aarch64".to_string(),
            created_at_unix_seconds: 100,
            last_seen_unix_seconds: 120,
        };

        db.upsert_fabric_root_identity(&root).unwrap();
        db.upsert_device_public_identity(&device).unwrap();

        assert_eq!(
            db.get_fabric_root_identity(&root.fabric_id).unwrap(),
            Some(root)
        );
        assert_eq!(
            db.get_device_public_identity(&device.device_id).unwrap(),
            Some(device)
        );
    }

    #[test]
    fn records_and_filters_audit_events() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();
        let mut first = AuditEventInput::new(
            AuditEventType::SecurityBlocked,
            AuditOutcome::Blocked,
            "blocked secret transfer",
        )
        .with_detail(serde_json::json!({
            "path": "/Users/me/project/.env",
            "api_token": "secret-token",
        }));
        first.project_id = Some("project-a".to_string());
        first.actor_device_id = Some("device-a".to_string());

        let mut second = AuditEventInput::new(
            AuditEventType::CommandApproved,
            AuditOutcome::Succeeded,
            "approved bootstrap command",
        );
        second.project_id = Some("project-b".to_string());

        let first = db.record_audit_event_at(first, 100).unwrap();
        let second = db.record_audit_event_at(second, 101).unwrap();

        assert_eq!(first.schema_version, AUDIT_SCHEMA_VERSION);
        assert_eq!(first.event_type, AuditEventType::SecurityBlocked);
        assert_eq!(first.outcome, AuditOutcome::Blocked);
        assert_eq!(first.detail["api_token"], "secret-token");

        assert_eq!(
            db.list_audit_events(None, 10)
                .unwrap()
                .iter()
                .map(|event| event.audit_id)
                .collect::<Vec<_>>(),
            vec![second.audit_id, first.audit_id]
        );
        assert_eq!(
            db.list_audit_events(Some("project-a"), 10).unwrap(),
            vec![first]
        );
    }

    #[test]
    fn revokes_device_and_records_audit_event() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let mut db = MetadataDb::open(&path).unwrap();

        let revocation = db
            .revoke_device_at("device-b", "device-a", "lost laptop", true, 123)
            .unwrap();

        assert_eq!(revocation.device_id, "device-b");
        assert_eq!(revocation.revoked_by_device_id, "device-a");
        assert_eq!(revocation.reason, "lost laptop");
        assert!(revocation.key_rotation_required);
        assert_eq!(
            db.get_device_revocation("device-b").unwrap(),
            Some(revocation.clone())
        );
        assert!(
            db.ensure_device_not_revoked("device-b", "test operation")
                .unwrap_err()
                .to_string()
                .contains("test operation rejected")
        );

        let audits = db.list_audit_events(None, 10).unwrap();
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, AuditEventType::DeviceRevoked);
        assert_eq!(audits[0].actor_device_id.as_deref(), Some("device-a"));
        assert_eq!(audits[0].target_device_id.as_deref(), Some("device-b"));
        assert_eq!(audits[0].detail["key_rotation_required"], true);
    }

    fn setup_pairing_db() -> (MetadataDb, FabricRootIdentity) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.keep().join("metadata.sqlite");
        let db = MetadataDb::open(&path).unwrap();
        let root = FabricRootIdentity {
            fabric_id: "f_1234567890abcdef12345678".to_string(),
            fabric_name: "Test Fabric".to_string(),
            root_public_key_hex: "a".repeat(64),
            created_at_unix_seconds: 100,
            rotation_epoch: 0,
        };
        db.upsert_fabric_root_identity(&root).unwrap();
        (db, root)
    }

    fn pairing_start_request(root: &FabricRootIdentity) -> PairingStartRequest<'_> {
        PairingStartRequest {
            fabric_id: &root.fabric_id,
            local_device_id: "d_local",
            peer_device_id: "d_peer",
            peer_display_name: "Peer Laptop",
            peer_signing_public_key_hex: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            peer_network_public_key_hex: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            peer_ephemeral_public_key_hex: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            anchor_address: Some("192.0.2.1:7000"),
            ttl_seconds: 60,
        }
    }

    #[test]
    fn pairing_session_confirms_and_persists_paired_device() {
        let (mut db, root) = setup_pairing_db();
        let session = db
            .start_pairing_session(pairing_start_request(&root))
            .unwrap();
        assert!(session.pairing_id.starts_with("pa_"));
        assert_eq!(session.state, PairingState::Pending);
        assert_eq!(session.anchor_address.as_deref(), Some("192.0.2.1:7000"));
        assert_eq!(session.short_authentication_string.len(), 6);

        let confirmed = db
            .confirm_pairing_session(
                &session.pairing_id,
                &session.short_authentication_string,
                r#"{"certificate":"issued"}"#,
                session.created_at_unix_seconds + 1,
            )
            .unwrap();
        assert_eq!(confirmed.state, PairingState::Confirmed);
        assert!(confirmed.confirmed_at_unix_seconds.is_some());
        assert_eq!(
            db.get_device_public_identity("d_peer")
                .unwrap()
                .unwrap()
                .display_name,
            "Peer Laptop"
        );
        let audits = db.list_audit_events(None, 10).unwrap();
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, AuditEventType::DevicePaired);
        assert_eq!(audits[0].actor_device_id.as_deref(), Some("d_local"));
        assert_eq!(audits[0].target_device_id.as_deref(), Some("d_peer"));
        assert_eq!(audits[0].detail["pairing_id"], session.pairing_id);
    }

    #[test]
    fn pairing_replay_cannot_confirm_twice() {
        let (mut db, root) = setup_pairing_db();
        let session = db
            .start_pairing_session(pairing_start_request(&root))
            .unwrap();
        db.confirm_pairing_session(
            &session.pairing_id,
            &session.short_authentication_string,
            r#"{"certificate":"issued"}"#,
            session.created_at_unix_seconds + 1,
        )
        .unwrap();

        let err = db
            .confirm_pairing_session(
                &session.pairing_id,
                &session.short_authentication_string,
                r#"{"certificate":"issued"}"#,
                session.created_at_unix_seconds + 2,
            )
            .unwrap_err();
        assert!(err.to_string().contains("already confirmed"));
    }

    #[test]
    fn pairing_mismatched_code_does_not_persist_device() {
        let (mut db, root) = setup_pairing_db();
        let session = db
            .start_pairing_session(pairing_start_request(&root))
            .unwrap();
        let wrong_code = if session.short_authentication_string == "000000" {
            "111111"
        } else {
            "000000"
        };

        let err = db
            .confirm_pairing_session(
                &session.pairing_id,
                wrong_code,
                r#"{"certificate":"issued"}"#,
                session.created_at_unix_seconds + 1,
            )
            .unwrap_err();
        assert!(err.to_string().contains("code mismatch"));
        assert_eq!(
            db.get_pairing_session(&session.pairing_id)
                .unwrap()
                .unwrap()
                .state,
            PairingState::Pending
        );
        assert!(db.get_device_public_identity("d_peer").unwrap().is_none());
    }

    #[test]
    fn pairing_expire_and_abort_paths_are_terminal() {
        let (mut db, root) = setup_pairing_db();
        let session = db
            .start_pairing_session(pairing_start_request(&root))
            .unwrap();
        let expired = db
            .expire_pairing_sessions(session.expires_at_unix_seconds + 1)
            .unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].state, PairingState::Expired);

        let session = db
            .start_pairing_session(pairing_start_request(&root))
            .unwrap();
        let aborted = db
            .abort_pairing_session(&session.pairing_id, session.created_at_unix_seconds + 1)
            .unwrap();
        assert_eq!(aborted.state, PairingState::Aborted);
        assert!(aborted.aborted_at_unix_seconds.is_some());
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

    fn remove_cas_chunk(cas: &CasStore, hash: &CasChunkHash) {
        let hex = hash.as_str().strip_prefix(CAS_HASH_PREFIX).unwrap();
        fs::remove_file(
            cas.root()
                .join("chunks")
                .join("b3")
                .join(&hex[0..2])
                .join(format!("{}.chunk", &hex[2..])),
        )
        .unwrap();
    }

    fn setup_publish_db_at(
        path: &std::path::Path,
        epoch: u64,
        state: LeaseState,
    ) -> (MetadataDb, StoredSession, LeaseRecord) {
        let db = MetadataDb::open(path).unwrap();
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

    fn setup_publish_db(epoch: u64, state: LeaseState) -> (MetadataDb, StoredSession, LeaseRecord) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.keep().join("metadata.sqlite");
        setup_publish_db_at(&path, epoch, state)
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
        let audits = db.list_audit_events(Some("project123"), 10).unwrap();
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, AuditEventType::SnapshotPublished);
        assert_eq!(audits[0].outcome, AuditOutcome::Succeeded);
        assert_eq!(
            audits[0].snapshot_id.as_deref(),
            Some(metadata.snapshot_id.as_str())
        );
        assert_eq!(audits[0].lease_id.as_deref(), Some("lease-1"));
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
    fn revoked_devices_cannot_publish_or_receive_handoff() {
        let (mut db, session, lease) = setup_publish_db(1, LeaseState::Active);
        db.revoke_device_at("device-a", "device-security", "compromised", false, 200)
            .unwrap();
        let metadata = publish_metadata("s1_000000000000000000000110", &session.session_id);

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

        assert!(err.to_string().contains("snapshot publish rejected"));
        assert!(err.to_string().contains("device-a"));
        assert!(snapshots_for_project(&db, "project123").is_empty());

        let (mut db, _session, lease) = setup_publish_db(1, LeaseState::Active);
        db.revoke_device_at("device-b", "device-security", "lost laptop", true, 201)
            .unwrap();
        let err = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
            .unwrap_err();
        assert!(err.to_string().contains("begin handoff rejected"));
        assert!(err.to_string().contains("device-b"));
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
    fn inactive_publish_creates_fork_without_latest_change() {
        let (mut db, session, lease) = setup_publish_db(1, LeaseState::Active);
        let canonical = publish_metadata("s1_000000000000000000000107", &session.session_id);
        db.publish_snapshot_canonical(CanonicalPublishRequest {
            lease_id: &lease.lease_id,
            session_id: &session.session_id,
            expected_epoch: 1,
            holder_device_id: "device-a",
            expected_latest_snapshot_id: None,
            metadata: &canonical,
            pinned: false,
            label: Some("canonical"),
        })
        .unwrap();

        let mut inactive = db.get_lease(&lease.lease_id).unwrap().unwrap();
        inactive.state = LeaseState::Inactive;
        db.upsert_lease(&inactive).unwrap();

        let edit = publish_metadata("s1_000000000000000000000108", &session.session_id);
        let fork = db
            .publish_inactive_snapshot_as_fork(InactiveForkPublishRequest {
                lease_id: &lease.lease_id,
                session_id: &session.session_id,
                holder_device_id: "device-a",
                metadata: &edit,
                label: None,
            })
            .unwrap();

        assert_eq!(fork.fork_session.state, SessionState::Fork);
        assert_eq!(
            fork.fork_session.parent_session_id.as_deref(),
            Some(session.session_id.as_str())
        );
        assert_eq!(
            fork.canonical_latest_snapshot_id.as_deref(),
            Some(canonical.snapshot_id.as_str())
        );
        assert!(fork.snapshot.pinned);
        assert_eq!(
            fork.snapshot.session_id.as_deref(),
            Some(fork.fork_session.session_id.as_str())
        );
        assert_eq!(
            fork.snapshot.parent_snapshot_id.as_deref(),
            Some(canonical.snapshot_id.as_str())
        );
        assert_eq!(
            snapshots_for_project(&db, "project123"),
            vec![canonical.snapshot_id.clone(), edit.snapshot_id.clone()]
        );

        let updated = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(updated.state, LeaseState::Forked);
        assert_eq!(
            updated.latest_snapshot_id.as_deref(),
            Some(canonical.snapshot_id.as_str())
        );
    }

    #[test]
    fn inactive_fork_snapshot_is_recoverable_from_store() {
        let (mut db, session, lease) = setup_publish_db(1, LeaseState::Inactive);
        let edit = publish_metadata("s1_000000000000000000000109", &session.session_id);
        let fork = db
            .publish_inactive_snapshot_as_fork(InactiveForkPublishRequest {
                lease_id: &lease.lease_id,
                session_id: &session.session_id,
                holder_device_id: "device-a",
                metadata: &edit,
                label: Some("separate work"),
            })
            .unwrap();

        let stored = snapshot_from_store(&db, &fork.snapshot.snapshot_id);
        assert_eq!(stored.snapshot_id, fork.snapshot.snapshot_id);
        assert_eq!(
            stored.session_id.as_deref(),
            Some(fork.fork_session.session_id.as_str())
        );
        assert_eq!(
            stored.metadata.session_id.as_deref(),
            Some(fork.fork_session.session_id.as_str())
        );
        assert_eq!(stored.label.as_deref(), Some("separate work"));
        assert!(stored.pinned);
        assert!(
            db.get_session(&fork.fork_session.session_id)
                .unwrap()
                .is_some()
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
        let lease_audits = db
            .list_audit_events(Some("project123"), 10)
            .unwrap()
            .into_iter()
            .filter(|event| event.event_type == AuditEventType::LeaseTransferred)
            .collect::<Vec<_>>();
        assert_eq!(lease_audits.len(), 1);
        assert_eq!(lease_audits[0].actor_device_id.as_deref(), Some("device-a"));
        assert_eq!(
            lease_audits[0].target_device_id.as_deref(),
            Some("device-b")
        );
        assert_eq!(
            lease_audits[0].handoff_id.as_deref(),
            Some(handoff.handoff_id.as_str())
        );
    }

    #[test]
    fn handoff_snapshot_preflight_blocks_missing_chunk_before_lease_transfer() {
        let temp = tempfile::tempdir().unwrap();
        let target_root = temp.path().join("target");
        fs::create_dir_all(&target_root).unwrap();
        let cas = CasStore::open(temp.path().join("cas")).unwrap();
        let sidecar_bytes = b"large handoff payload";
        let chunk = CasChunkHash::from_bytes(sidecar_bytes);
        cas.upload_chunk(sidecar_bytes, &chunk).unwrap();
        let manifest = cas.create_manifest(std::slice::from_ref(&chunk)).unwrap();
        let (mut db, session, lease) = setup_publish_db(14, LeaseState::Active);
        let mut metadata = publish_metadata("s1_000000000000000000000202", &session.session_id);
        metadata.sidecars = vec![SnapshotSidecar {
            logical_path: "large.bin".to_string(),
            file_mode: "100644".to_string(),
            classification: classification_reason::LARGE_FILE_THRESHOLD.to_string(),
            size_bytes: sidecar_bytes.len() as u64,
            chunk_size_bytes: DEFAULT_SIDECAR_CHUNK_BYTES as u64,
            root_hash: manifest.manifest_id.clone(),
            cas_manifest_id: manifest.manifest_id,
        }];

        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
            .unwrap();
        db.mark_handoff_target_verified(&handoff.handoff_id)
            .unwrap();
        db.mark_handoff_source_ready(&handoff.handoff_id).unwrap();
        remove_cas_chunk(&cas, &chunk);

        let err = db
            .commit_handoff_with_snapshot_preflight(
                &handoff.handoff_id,
                "gen-1",
                handoff.expires_at_unix_seconds - 1,
                HandoffCommitSnapshotPreflight {
                    target_repo_root: &target_root,
                    snapshot: &metadata,
                    cas_store: &cas,
                },
            )
            .unwrap_err();

        assert!(err.to_string().contains("missing 1 CAS chunks"));
        let unchanged = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(unchanged.state, LeaseState::HandoffPending);
        assert_eq!(unchanged.epoch, 14);
        assert_eq!(unchanged.holder_device_id.as_deref(), Some("device-a"));
        assert_eq!(
            unchanged.handoff_id.as_deref(),
            Some(handoff.handoff_id.as_str())
        );
        assert_eq!(
            db.get_handoff(&handoff.handoff_id).unwrap().unwrap().state,
            HandoffState::SourceReady
        );
        assert!(
            !handoff_journal_phases(&db, &handoff.handoff_id)
                .contains(&HandoffJournalPhase::LeaseCommitted)
        );
        assert!(
            db.list_audit_events(Some("project123"), 100)
                .unwrap()
                .iter()
                .all(|event| event.event_type != AuditEventType::LeaseTransferred)
        );

        cas.upload_chunk(sidecar_bytes, &chunk).unwrap();
        db.commit_handoff_with_snapshot_preflight(
            &handoff.handoff_id,
            "gen-1",
            handoff.expires_at_unix_seconds - 1,
            HandoffCommitSnapshotPreflight {
                target_repo_root: &target_root,
                snapshot: &metadata,
                cas_store: &cas,
            },
        )
        .unwrap();
        let committed = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(committed.state, LeaseState::Active);
        assert_eq!(committed.epoch, 15);
        assert_eq!(committed.holder_device_id.as_deref(), Some("device-b"));
    }

    #[test]
    fn handoff_commit_fault_rolls_back_lease_transfer() {
        let (mut db, _session, lease) = setup_publish_db(12, LeaseState::Active);
        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
            .unwrap();
        db.mark_handoff_target_verified(&handoff.handoff_id)
            .unwrap();
        db.mark_handoff_source_ready(&handoff.handoff_id).unwrap();
        db.set_fault_injection(Some(MetadataDbFaultPoint::DuringLeaseCommit));

        let err = db
            .commit_handoff(
                &handoff.handoff_id,
                "gen-1",
                handoff.expires_at_unix_seconds - 1,
            )
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("injected metadata DB fault at during-lease-commit")
        );
        let unchanged = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(unchanged.state, LeaseState::HandoffPending);
        assert_eq!(unchanged.epoch, 12);
        assert_eq!(unchanged.holder_device_id.as_deref(), Some("device-a"));
        assert_eq!(
            unchanged.handoff_id.as_deref(),
            Some(handoff.handoff_id.as_str())
        );
        assert_eq!(
            db.get_handoff(&handoff.handoff_id).unwrap().unwrap().state,
            HandoffState::SourceReady
        );
        assert!(
            !handoff_journal_phases(&db, &handoff.handoff_id)
                .contains(&HandoffJournalPhase::LeaseCommitted)
        );
        assert!(
            db.list_audit_events(Some("project123"), 100)
                .unwrap()
                .iter()
                .all(|event| event.event_type != AuditEventType::LeaseTransferred)
        );

        db.set_fault_injection(None);
        db.commit_handoff(
            &handoff.handoff_id,
            "gen-1",
            handoff.expires_at_unix_seconds - 1,
        )
        .unwrap();
        let committed = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(committed.state, LeaseState::Active);
        assert_eq!(committed.epoch, 13);
        assert_eq!(committed.holder_device_id.as_deref(), Some("device-b"));
    }

    #[test]
    fn handoff_journal_records_lifecycle_phases() {
        let (mut db, _session, lease) = setup_publish_db(8, LeaseState::Active);
        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
            .unwrap();

        let apply = db
            .record_handoff_target_apply(
                &handoff.handoff_id,
                Some(r#"{"snapshot_id":"s1_000000000000000000000201"}"#),
            )
            .unwrap();
        assert_eq!(apply.phase, HandoffJournalPhase::TargetApply);
        assert!(apply.detail_json.contains("snapshot_id"));

        db.mark_handoff_target_verified(&handoff.handoff_id)
            .unwrap();
        db.mark_handoff_source_ready(&handoff.handoff_id).unwrap();
        db.commit_handoff(
            &handoff.handoff_id,
            "gen-1",
            handoff.expires_at_unix_seconds - 1,
        )
        .unwrap();

        assert_eq!(
            handoff_journal_phases(&db, &handoff.handoff_id),
            vec![
                HandoffJournalPhase::Begin,
                HandoffJournalPhase::TargetPrepare,
                HandoffJournalPhase::TargetApply,
                HandoffJournalPhase::TargetVerified,
                HandoffJournalPhase::SourceReady,
                HandoffJournalPhase::LeaseCommitted,
            ]
        );
    }

    #[test]
    fn recover_handoff_commits_after_crash_before_commit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let (handoff_id, lease_id, expires_at) = {
            let (mut db, _session, lease) = setup_publish_db_at(&path, 9, LeaseState::Active);
            let handoff = db
                .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
                .unwrap();
            db.record_handoff_target_apply(&handoff.handoff_id, None)
                .unwrap();
            db.mark_handoff_target_verified(&handoff.handoff_id)
                .unwrap();
            db.mark_handoff_source_ready(&handoff.handoff_id).unwrap();
            (
                handoff.handoff_id.clone(),
                lease.lease_id.clone(),
                handoff.expires_at_unix_seconds,
            )
        };

        let mut db = MetadataDb::open(&path).unwrap();
        let outcome = db
            .recover_handoff(&handoff_id, "gen-1", expires_at - 1)
            .unwrap();
        assert_eq!(outcome, HandoffRecoveryOutcome::Committed);

        let updated = db.get_lease(&lease_id).unwrap().unwrap();
        assert_eq!(updated.state, LeaseState::Active);
        assert_eq!(updated.epoch, 10);
        assert_eq!(updated.holder_device_id.as_deref(), Some("device-b"));
        assert_eq!(updated.handoff_id, None);
        assert!(
            handoff_journal_phases(&db, &handoff_id).contains(&HandoffJournalPhase::LeaseCommitted)
        );
    }

    #[test]
    fn recover_handoff_aborts_expired_incomplete_handoff() {
        let (mut db, _session, lease) = setup_publish_db(10, LeaseState::Active);
        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 1)
            .unwrap();

        let outcome = db
            .recover_handoff(
                &handoff.handoff_id,
                "gen-1",
                handoff.expires_at_unix_seconds + 1,
            )
            .unwrap();
        assert_eq!(outcome, HandoffRecoveryOutcome::AbortedExpired);
        assert_eq!(
            db.get_handoff(&handoff.handoff_id).unwrap().unwrap().state,
            HandoffState::Aborted
        );
        let updated = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(updated.state, LeaseState::Active);
        assert_eq!(updated.epoch, 10);
        assert_eq!(updated.holder_device_id.as_deref(), Some("device-a"));
        assert_eq!(updated.handoff_id, None);
        assert!(
            handoff_journal_phases(&db, &handoff.handoff_id)
                .contains(&HandoffJournalPhase::Aborted)
        );
    }

    #[test]
    fn recover_handoff_is_idempotent_after_commit_crash() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite");
        let (handoff_id, lease_id, expires_at) = {
            let (mut db, _session, lease) = setup_publish_db_at(&path, 11, LeaseState::Active);
            let handoff = db
                .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 60)
                .unwrap();
            db.mark_handoff_target_verified(&handoff.handoff_id)
                .unwrap();
            db.mark_handoff_source_ready(&handoff.handoff_id).unwrap();
            db.commit_handoff(
                &handoff.handoff_id,
                "gen-1",
                handoff.expires_at_unix_seconds - 1,
            )
            .unwrap();
            (
                handoff.handoff_id.clone(),
                lease.lease_id.clone(),
                handoff.expires_at_unix_seconds,
            )
        };

        let mut db = MetadataDb::open(&path).unwrap();
        let outcome = db
            .recover_handoff(&handoff_id, "gen-1", expires_at - 1)
            .unwrap();
        assert_eq!(outcome, HandoffRecoveryOutcome::AlreadyCommitted);

        let updated = db.get_lease(&lease_id).unwrap().unwrap();
        assert_eq!(updated.state, LeaseState::Active);
        assert_eq!(updated.epoch, 12);
        assert_eq!(updated.holder_device_id.as_deref(), Some("device-b"));
        assert_eq!(
            handoff_journal_phases(&db, &handoff_id)
                .into_iter()
                .filter(|phase| *phase == HandoffJournalPhase::LeaseCommitted)
                .count(),
            1
        );
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

    fn snapshot_from_store(db: &MetadataDb, snapshot_id: &str) -> StoredSnapshot {
        struct SnapshotRow {
            snapshot_id: String,
            project_id: String,
            session_id: Option<String>,
            parent_snapshot_id: Option<String>,
            sequence_number: i64,
            pinned: bool,
            label: Option<String>,
            metadata_json: String,
            created_at_unix_seconds: i64,
        }

        let row: SnapshotRow = db
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
WHERE snapshot_id = ?1
"#,
                [snapshot_id],
                |row| {
                    Ok(SnapshotRow {
                        snapshot_id: row.get(0)?,
                        project_id: row.get(1)?,
                        session_id: row.get(2)?,
                        parent_snapshot_id: row.get(3)?,
                        sequence_number: row.get(4)?,
                        pinned: row.get(5)?,
                        label: row.get(6)?,
                        metadata_json: row.get(7)?,
                        created_at_unix_seconds: row.get(8)?,
                    })
                },
            )
            .unwrap();

        StoredSnapshot {
            snapshot_id: row.snapshot_id,
            project_id: row.project_id,
            session_id: row.session_id,
            parent_snapshot_id: row.parent_snapshot_id,
            sequence_number: row.sequence_number,
            pinned: row.pinned,
            label: row.label,
            metadata: serde_json::from_str(&row.metadata_json).unwrap(),
            created_at_unix_seconds: row.created_at_unix_seconds.max(0) as u64,
        }
    }

    fn handoff_journal_phases(db: &MetadataDb, handoff_id: &str) -> Vec<HandoffJournalPhase> {
        db.list_handoff_journal(handoff_id)
            .unwrap()
            .into_iter()
            .map(|record| record.phase)
            .collect()
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
        assert_eq!(count, 9);
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
