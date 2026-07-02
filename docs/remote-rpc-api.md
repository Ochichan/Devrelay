# Remote Control RPC API

Last updated: 2026-07-02

Status: implemented. The agent serves this API over mTLS when started with
`--remote-listen`, every method in the allowlist below is dispatched, and the
boundary is covered by agent integration tests
(`crates/devrelay-agent/tests/remote_rpc.rs`) plus core unit tests for the
authentication and preflight paths.

ADR 0005 selects JSON-RPC 2.0 over mTLS for the remote Control API. This
document defines the remote method allowlist and schema rules for protocol
version 1.

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

The agent binds the listener given by `devrelay-agent --remote-listen
<ip:port>` (port `0` selects an ephemeral port) and writes the bound address to
`DEVRELAY_HOME/agent-remote.addr`; `agent.health` reports it as
`remote_listen_address`. The TLS server identity is the device leaf
certificate issued by the deterministic fabric X.509 CA, and client
certificates must chain to that same CA. Both leaf certificates carry the
device's ed25519 signing key, which is also recorded in the fabric-signed
application-level device certificate.

Before method dispatch, the server verifies:

- TLS is active and the client certificate chains to the fabric X.509 CA.
- The application-level device certificate in the request frame validates
  against the pinned fabric root (issuer, signature, validity window).
- The device is not revoked; revocation state is reloaded per request and
  failures to load it fail closed.
- The TLS peer leaf public key equals the device certificate's signing key,
  binding the channel to the claimed device.
- Request timestamp is inside the clock-skew window.
- Replay nonce has not been used inside the replay window.
- JSON-RPC request ID is present and the method is in the allowlist.

Rejected requests receive a mapped JSON-RPC error response and are recorded as
`security.blocked` audit events on the serving agent.

Remote RPC is not local IPC. Local-only methods such as editor context,
settings mutation, diagnostics export, metrics export, and direct filesystem
status are not in the remote allowlist.

## Wire Framing

Each request is one length-prefixed (u32 big-endian) JSON frame:

```json
{
  "control": {
    "protocol_version": 1,
    "sent_at_unix_seconds": 1710000000,
    "replay_nonce": "f3a09c2d5b1e48d7a6c40b91e2d35f80"
  },
  "device_certificate": { "certificate_id": "cert_..." },
  "rpc": { "jsonrpc": "2.0", "id": "client-1", "method": "devices.list", "params": {} }
}
```

Responses are plain length-prefixed JSON-RPC 2.0 responses. A connection may
carry many sequential requests; it stays usable after a rejected request.
Message size, connection, and request timeouts follow the shared control
transport policy (1 MiB, 10 s, 30 s by default).

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
