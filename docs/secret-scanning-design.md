# Secret Scanning Override And One-Time Sidecar Design

Last updated: 2026-06-24

This design keeps the default rule unchanged: secret-like content is excluded
from snapshots unless the user makes a narrow, auditable exception.

## Goals

- Avoid blocking legitimate repositories forever because of known false
  positives such as fixtures, public test keys, or dummy tokens.
- Keep overrides narrow enough that a malicious manifest cannot silently
  whitelist real secrets.
- Provide a path for one-time transfer of explicitly selected local-only secret
  material without storing plaintext in snapshot metadata, Git objects, CAS, or
  diagnostics.

## False-Positive Override Design

### Override Types

DevRelay should support two override classes:

- **Local approval:** stored in local metadata DB, scoped to project, device,
  relative path, scanner reason, and content hash. This is the default UI path.
- **Portable manifest allowlist:** stored in the project manifest for intentional
  shared fixtures. Any manifest allowlist change must affect the execution
  trust hash and require command trust approval before it is honored.

### Local Approval Record

A local approval record should contain:

- `project_id`
- `device_id`
- `relative_path`
- scanner reason code, such as `secret-filename`, `private-key-content`, or
  `high-entropy-secret`
- normalized content hash for the exact file bytes or symlink target string
- decision: `allow-once`, `trust-this-content`, or `reject`
- human reason text
- creation time, optional expiry time, optional consumed time

The scanner may include a file only when the current path, reason code, and
content hash match an unexpired approval. A path-only approval is not enough.

### Portable Allowlist Rules

Portable allowlists are for repository-owned fixtures, not local secrets. They
must be explicit and reviewable:

```toml
[workspace.secret_scanner.allowlist."tests/fixtures/public-test-key.pem"]
reason = "public fixture key used by parser tests"
detectors = ["private-key-content"]
content_hash = "blake3:..."
```

Rules:

- `content_hash` is required.
- Absolute paths and `..` are rejected.
- Manifest-declared secret targets cannot be allowlisted.
- Private user paths under `.ssh`, `.config`, `.aws`, `.kube`, and
  `.devrelay/secrets` cannot be allowlisted.
- Allowlist entries must never disable diagnostic/log redaction.

### User Flow

When a scan blocks a suspected false positive, the UI/CLI should show:

- exact relative path
- scanner reason and detector details
- content hash
- whether the file is tracked, accepted untracked, or ignored
- choices: `allow once`, `trust this content`, `reject`, `open file`, `exclude`

Approvals must be auditable and revocable. `allow once` is consumed on the next
successful snapshot that includes the file.

### Safety Properties

- A changed file requires new approval.
- A renamed file requires new approval unless the portable allowlist explicitly
  covers that path and content hash.
- Overrides do not affect log redaction, diagnostic redaction, command trust,
  task cache secret-sensitivity, or manifest secret target hard exclusions.
- Background checkpointing may use only existing non-expired approvals; it must
  not create approvals.

## Encrypted One-Time Sidecar Design

One-time sidecars are for explicit handoff of local-only material that should be
available on a selected target once, without becoming normal snapshot content.

### Envelope

Each encrypted one-time sidecar should include:

- `sidecar_id`
- source `project_id`, source `session_id`, source `snapshot_id`
- source and target device IDs
- content type and relative materialization path
- plaintext content hash and byte length
- expiry time and max materialization count
- AEAD algorithm and nonce
- encrypted payload or CAS root for encrypted chunks
- encrypted content key for the target device
- source device signature over envelope metadata

The initial algorithm should be XChaCha20-Poly1305 or AES-256-GCM with a fresh
random content key per sidecar. The content key should be sealed to the target
device using the existing paired-device public key material or a derived
control-plane key.

### Transfer Rules

- Requires explicit user action naming the target device and path.
- May be attached only to a handoff or recovery operation, not to background
  checkpoints.
- Ciphertext may be stored in CAS/anchor as evictable sidecar data; plaintext
  must never be stored in Git objects, snapshot metadata, logs, diagnostics, or
  task result cache.
- Target materializes plaintext only after certificate/revocation checks,
  snapshot verification, and sidecar envelope verification pass.
- On success, the target records materialization and the source/anchor can
  delete the encrypted reachability root after retention grace.

### Failure Behavior

- Expired, revoked-device, wrong-target, corrupt, missing-key, or replayed
  sidecars fail closed.
- Handoff must continue only if the sidecar is optional; required sidecar
  failure blocks before target mutation.
- No fallback may write plaintext into the normal snapshot or task artifact
  paths.

### Audit And UX

Audit events should record:

- source and target device IDs
- sidecar ID
- destination relative path
- content hash, byte length, expiry, and materialization result

UX copy should describe this as "send once to this device" rather than
"encrypted sidecar". It must make clear that the receiving device can read the
plaintext after materialization.

### Non-Goals

- No cross-target broadcast.
- No deduplication of encrypted secret payloads.
- No automatic transfer of all local secret providers.
- No use as a replacement for a real secret manager.

## Implementation Hooks

- Secret scanner and hard exclusions: `crates/devrelay-core/src/policy.rs`,
  `crates/devrelay-core/src/secret_provider.rs`.
- Sidecar capture/materialization: `crates/devrelay-core/src/sidecar.rs`.
- CAS encrypted chunk storage: `crates/devrelay-core/src/cas.rs`.
- Handoff preflight and lease transfer: `crates/devrelay-core/src/storage.rs`.
- Transport identity and revocation checks:
  `crates/devrelay-core/src/transport_security.rs`.
