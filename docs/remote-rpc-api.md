# Remote Control RPC API

Last updated: 2026-06-24

Status: schema plan, core pre-dispatch policy, and core read handlers for
`devices.list`, `projects.list`, `workspaces.list`, and
`sessions.snapshots.list`, plus role-gated core handoff handlers, accepted;
server not implemented.

ADR 0005 selects JSON-RPC 2.0 over mTLS for the remote Control API. This
document defines the first remote method allowlist and schema rules required
before M4.5 can close.

The core pre-dispatch helper enforces the method allowlist, authenticated mTLS
peer requirement, control-envelope validation, request ID requirement, and JSON
error mapping. A remote socket server still has to call that helper before
method dispatch.

The core read handlers for `devices.list`, `projects.list`,
`workspaces.list`, and `sessions.snapshots.list` return remote-safe data only.
Project and workspace responses do not serialize local filesystem paths, and
snapshot list responses do not serialize full snapshot metadata.

The core handoff handlers require the authenticated actor device to match the
expected handoff role before mutating state: source for `handoff.begin`,
`handoff.source.ready`, and `handoff.commit`; target for
`handoff.target.verify`; and source or target for `handoff.abort` and
`handoff.recover`.

## Transport And Auth

Remote Control RPC runs only over the mTLS control transport.

Before method dispatch, the server must verify:

- TLS is active.
- The client certificate chains to the pinned fabric root.
- The client device is not revoked.
- Protocol version negotiation has succeeded.
- Request timestamp is inside the clock-skew window.
- Replay nonce has not been used.
- JSON-RPC request ID is present and stable.

Remote RPC is not local IPC. Local-only methods such as editor context,
settings mutation, diagnostics export, metrics export, and direct filesystem
status are not in the first remote allowlist.

## Envelope

Requests use JSON-RPC 2.0:

```json
{
  "jsonrpc": "2.0",
  "id": "client-1",
  "method": "devices.list",
  "params": {}
}
```

Successful responses echo the request ID:

```json
{
  "jsonrpc": "2.0",
  "id": "client-1",
  "result": {}
}
```

Errors use the same JSON-RPC error envelope as local RPC. DevRelay-specific
details live under `error.data`; `error.message` is a stable summary.

## Naming Rules

- Method names are dot-separated verbs.
- JSON fields use `snake_case`.
- Additive optional params and result fields are allowed within protocol
  version `1`.
- Removing or renaming a method, required param, or result field requires a
  protocol version bump.

## Negotiation

### `rpc.negotiate`

Params:

```json
{ "client_protocol_version": 1 }
```

Result:

```json
{
  "protocol_version": 1,
  "server_name": "devrelay-remote",
  "methods": ["devices.list"]
}
```

## Device And Project Discovery

### `devices.list`

Params:

```json
{}
```

Result:

```json
{
  "devices": [
    {
      "device_id": "device-a",
      "display_name": "MacBook",
      "platform_key": "darwin-arm64",
      "architecture": "arm64",
      "capabilities_json": "{}",
      "paired_at_unix_seconds": 1710000000,
      "last_seen_unix_seconds": 1710000100
    }
  ]
}
```

### `projects.list`

Params:

```json
{}
```

Result:

```json
{
  "projects": [
    {
      "project_id": "12345678",
      "display_name": "Demo",
      "workspace_count": 2,
      "remote_url_fingerprint": "optional",
      "root_commit_fingerprint": "optional"
    }
  ]
}
```

Remote project lists must not expose local filesystem paths by default.

### `workspaces.list`

Params:

```json
{ "project": "12345678" }
```

Result:

```json
{
  "workspaces": [
    {
      "workspace_id": "ws_abc",
      "project_id": "12345678",
      "device_id": "device-a",
      "platform_profile": "darwin-arm64",
      "state": "active",
      "last_seen_head": "optional",
      "last_checkpoint_id": "optional",
      "local_path_redacted": true
    }
  ]
}
```

## Snapshots

### `sessions.snapshots.list`

Params:

```json
{
  "project": "12345678",
  "session_id": "se_optional",
  "limit": 50
}
```

Result:

```json
{
  "snapshots": [
    {
      "snapshot_id": "s1_0123456789abcdef01234567",
      "project_id": "12345678",
      "session_id": "se_optional",
      "parent_snapshot_id": null,
      "sequence_number": 1,
      "pinned": false,
      "label": "checkpoint",
      "created_at_unix_seconds": 1710000200
    }
  ]
}
```

The control method lists metadata. Git object, CAS, and sidecar transfer remain
data-plane operations.

When `limit` is omitted, the core handler returns up to 100 snapshots. Explicit
limits are capped at 500 and applied after sorting newest sequence first.

## Handoffs

### `handoffs.list`

Params:

```json
{
  "project": "12345678",
  "include_journal": true
}
```

Result:

```json
{
  "handoffs": [
    {
      "record": {
        "handoff_id": "ho_abc",
        "lease_id": "lease-1",
        "project_id": "12345678",
        "expected_epoch": 2,
        "source_device_id": "device-a",
        "target_device_id": "device-b",
        "source_generation": "gen-1",
        "expires_at_unix_seconds": 1710000300,
        "state": "target-prepare"
      },
      "journal": []
    }
  ]
}
```

### `handoff.begin`

Params:

```json
{
  "project": "12345678",
  "lease_id": "lease-1",
  "target_device_id": "device-b",
  "source_generation": "gen-1",
  "ttl_seconds": 600
}
```

Result: `{ "handoff": HandoffRecord, "journal": HandoffJournalRecord[] }`.

### `handoff.target.verify`

Params:

```json
{ "project": "12345678", "handoff_id": "ho_abc" }
```

Result: `{ "handoff": HandoffRecord, "journal": HandoffJournalRecord[] }`.

### `handoff.source.ready`

Params:

```json
{ "project": "12345678", "handoff_id": "ho_abc" }
```

Result: `{ "handoff": HandoffRecord, "journal": HandoffJournalRecord[] }`.

### `handoff.commit`

Params:

```json
{
  "project": "12345678",
  "handoff_id": "ho_abc",
  "observed_source_generation": "gen-1"
}
```

Result: `{ "handoff": HandoffRecord, "journal": HandoffJournalRecord[] }`.

### `handoff.abort`

Params:

```json
{ "project": "12345678", "handoff_id": "ho_abc" }
```

Result: `{ "handoff": HandoffRecord, "journal": HandoffJournalRecord[] }`.

### `handoff.recover`

Params:

```json
{
  "project": "12345678",
  "handoff_id": "ho_abc",
  "observed_source_generation": "gen-1"
}
```

Result:

```json
{
  "outcome": "waiting-for-target",
  "handoff": {},
  "journal": []
}
```

Allowed `outcome` values are `waiting-for-target`, `committed`,
`aborted-expired`, `already-committed`, and `already-aborted`.

## Recovery

### `recovery.list`

Params:

```json
{ "project": "12345678", "limit": 50 }
```

`limit` is optional. When omitted, the core handler returns up to 100
snapshots. Explicit limits are capped at 500 and applied after sorting newest
sequence first, matching `sessions.snapshots.list`.

Result:

```json
{
  "snapshots": [
    {
      "snapshot_id": "s1_0123456789abcdef01234567",
      "project_id": "12345678",
      "session_id": "se_optional",
      "parent_snapshot_id": null,
      "sequence_number": 1,
      "pinned": true,
      "label": "dirty target backup",
      "created_at_unix_seconds": 1710000200
    }
  ]
}
```

### `recovery.open`

Params:

```json
{
  "snapshot_id": "s1_0123456789abcdef01234567",
  "project": "12345678",
  "path": "/path/on/receiving-device",
  "register": false,
  "name": "optional workspace name"
}
```

Result:

```json
{
  "recovered": {
    "snapshot_id": "s1_0123456789abcdef01234567",
    "project_id": "12345678",
    "session_id": "se_optional",
    "pinned": true,
    "label": "dirty target backup"
  },
  "path": "/path/on/receiving-device",
  "name": "optional workspace name",
  "registered": null,
  "verification": {
    "head_oid": "verified head",
    "index_tree_oid": "verified index tree",
    "work_tree_oid": "verified work tree",
    "state_hash": "verified state hash",
    "included_untracked": [],
    "excluded_paths": []
  }
}
```

`recovery.open` is executed on the receiving device. The path is local to that
device and must not be sent to unrelated peers or diagnostics unless explicitly
requested by the user.
