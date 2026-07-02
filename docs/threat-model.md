# DevRelay Threat Model

Last updated: 2026-07-02

This document records the current implementation-grounded threat model for
DevRelay. It is not a release security sign-off; it is the baseline that future
security reviews and beta gates should test against.

## Scope

Covered:

- Local agent, desktop shell, editor extension, CLI, and local metadata store.
- Pairing, device identity, mTLS transport primitives, revocation, and replay
  controls.
- Snapshot Git objects, CAS sidecars, anchor data-plane helpers, task runner
  workspaces, artifacts, and command trust hashes.
- Project manifests, environment hydration commands, secret materialization,
  diagnostic redaction, watcher-triggered checkpointing, and recovery paths.

Not fully covered:

- Real two-device manual boundary evidence for the remote Control API. The
  M4.5 boundary is implemented with integration tests; the manual verification
  runbook still gates broad LAN trust claims.
- Signed release/update supply chain.
- Backup anchor replication and restore.
- Independent third-party security review.

## Assets

- User workspace contents, including uncommitted changes and accepted
  untracked files.
- Snapshot metadata, Git object refs, CAS chunks, sidecar manifests, and task
  artifacts.
- Single-writer lease state, handoff state, session identity, and recovery
  records.
- Device identities, fabric root material, device certificates, revocation
  records, and audit events.
- Command trust approvals and manifest execution trust hashes.
- Local secrets and diagnostic bundles.

## Trust Boundaries

- **Local user boundary:** Unix socket IPC is local-user scoped on macOS/Linux.
  Windows named pipe ACL work is still open.
- **Paired device boundary:** remote peers must be paired and validated by
  certificate material before mTLS control traffic is accepted.
- **Data-plane boundary:** Git refs and CAS chunks are content addressed and
  namespace-constrained, but availability can still fail.
- **Manifest boundary:** project manifests and bootstrap commands are untrusted
  until their execution trust hash is approved.
- **Filesystem boundary:** watcher events are hints; scans and Git state decide
  canonical state.
- **UI boundary:** desktop/editor UI can display or request actions but must not
  compute canonical state independently from the agent.

## Accepted Trust Assumptions

- A fully compromised currently trusted device can act with that device's
  authority until the user revokes it. DevRelay can limit blast radius, preserve
  data, and audit events; it cannot make a compromised endpoint trustworthy.
- Local OS account isolation is trusted for local IPC and local secret files.
- A user approving a command trust hash is accepting execution of that exact
  executable manifest state for the scoped project/device command.
- The Git object model and cryptographic content hashes are trusted as integrity
  primitives, subject to DevRelay validating expected object IDs and state
  hashes before mutation.
- Remote storage or anchor infrastructure may be unavailable or stale; it must
  not be trusted to advance canonical session state by itself.

## Threats And Mitigations

| Threat | Attacker / Failure | Impact | Current Mitigations | Residual Risk |
| --- | --- | --- | --- | --- |
| Same-LAN attacker | Network peer can observe or attempt local-network traffic. | Unauthorized control requests, replay, or data fetch attempts. | Pairing transcript/SAS, mTLS with fabric-pinned certificates and TLS key binding, per-request device certificate and revocation checks, replay nonces, bounded clock skew, security-blocked audit records, namespace-limited Git refs. | Manual two-device boundary evidence pending before broad LAN trust claims. |
| Malicious project manifest | Repository changes manifest commands, bootstrap scripts, fingerprint files, secret targets, sidecars, or task definitions. | Unexpected command execution or secret/file escape. | Execution trust hash changes on executable edits and fingerprint-file content edits; native bootstrap requires trust approval; script profiles require trusted command scope; Dev Container image build/pull requires approval; manifest secret targets must stay inside workspace. | Trust prompt UX and false-positive override design remain open. |
| Stale device | Previously paired or offline device uses old state after lease/session changed. | Lost updates or stale canonical publish. | Lease epochs are monotonic; stale publishes are stored without advancing canonical latest; handoff source generation is checked; revocation blocks publish/handoff; replay nonce cache rejects reused control envelopes. | Revoked/old devices may still hold local copies; user-facing device trust doctor remains open. |
| Compromised device | Trusted endpoint is malicious after pairing. | It can request allowed operations until revoked. | Revocation records, mTLS certificate validation against revocation, audit events, dirty target blocking, immutable snapshot IDs, stale publish fork behavior, no background auto-merge. | Endpoint compromise cannot be fully mitigated locally; response depends on fast revocation and recovery. |
| Path traversal | Snapshot paths, artifact output paths, sidecar paths, secret targets, task run IDs, or schema child paths attempt `..` or absolute escape. | Overwrite/read outside intended workspace or metadata directories. | Snapshot schema rejects traversal; apply rejects path traversal before mutation; artifact output capture validates workspace-relative paths and task run IDs; secret and sidecar materialization normalize targets inside workspace. | Continue adding suite-level safety evidence for every new path-taking API. |
| Symlink escape | Symlink or symlink parent redirects scans/materialization outside workspace. | Secret disclosure or external file mutation. | Untracked symlink targets outside workspace are excluded; content checks do not follow symlink targets; sidecar materialization rejects symlink parent escape; targets without symlink support block symlink materialization. | Platform-specific filesystem behavior still needs real-device dogfood. |
| Secret leakage | Secret-like files, manifest-declared secret targets, command output, logs, diagnostics, or caches expose secret values. | Credential disclosure. | Secret scanner excludes default secret classes; manifest secret file targets are hard-excluded; local secret materialization produces redacted reports; logs and diagnostics are redacted by default; result cache disables secret-sensitive tasks by default. | False-positive override and encrypted one-time sidecar design remain open. |
| Replay attack | Reuse of pairing/control messages or stale envelopes. | Unauthorized repeated confirmation or control request. | Pairing confirmation cannot be replayed; control envelopes require protocol version, timestamp skew checks, safe nonce format, and replay-cache uniqueness. | Must be enforced by the final implemented remote API boundary. |
| Data corruption | Crash, partial upload, missing chunks, disk full, or malformed object data. | Invalid snapshot, broken recovery, or unsafe apply. | Snapshot metadata/state hash validation, immutable published snapshots, guarded publish uploads Git and CAS before advancing latest, handoff preflight checks chunks before lease transfer, CAS manifest verification, crash journal replay, fault injection tests, and pruning protections for latest/pinned/child snapshots. | Backup anchor replication/restore design remains open. |

## Implementation Review Map

- Pairing and replay controls: `crates/devrelay-core/src/pairing.rs`,
  `crates/devrelay-core/src/transport_security.rs`.
- Command trust and executable manifest hashing:
  `crates/devrelay-core/src/manifest.rs`,
  `crates/devrelay-core/src/storage.rs`,
  `crates/devrelay-core/src/environment.rs`,
  `crates/devrelay-core/src/task_runner_workspace.rs`.
- Snapshot and data-plane integrity:
  `crates/devrelay-core/src/snapshot.rs`,
  `crates/devrelay-core/src/snapshot_schema.rs`,
  `crates/devrelay-core/src/snapshot_upload.rs`,
  `crates/devrelay-core/src/cas.rs`,
  `crates/devrelay-core/src/data_plane.rs`.
- Secret and path defenses:
  `crates/devrelay-core/src/policy.rs`,
  `crates/devrelay-core/src/secret_provider.rs`,
  `crates/devrelay-core/src/sidecar.rs`,
  `crates/devrelay-core/src/task_artifacts.rs`,
  `crates/devrelay-core/src/logging.rs`.
- Safety evidence suites:
  `crates/devrelay-core/tests/safety.rs`,
  `crates/devrelay-core/tests/round_trip.rs`.

## Open Security Work

- Collect real-device manual boundary evidence for the remote Control API.
- Build command trust prompt and device trust doctor UX.
- Design safe false-positive overrides for secret scanning.
- Decide encrypted one-time sidecar and opaque anchor tradeoffs.
- Complete signed release/provenance strategy and independent security review.
