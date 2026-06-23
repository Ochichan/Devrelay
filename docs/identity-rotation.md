# Identity Rotation Design Note

Last updated: 2026-06-23

DevRelay M4.1 creates a dev-mode fabric root key, device signing key, and
network certificate key. Rotation is intentionally a later operation; this note
records the constraints so the first persisted schema does not block it.

M4 pairing, mTLS primitives, revocation records, and audit events exist, but
production trust still depends on recovery export, rotation UX, secure key
storage, and the unresolved M4.5 Control API boundary.

## Rotation Rules

- Fabric root rotation must create a new `fabric_id` and keep the previous root
  public identity until all paired devices have accepted the successor.
- Device signing-key rotation must preserve `device_id` and bump the public
  identity record atomically with a revocation record for the old key.
- Network certificate-key rotation may happen independently of the signing key,
  but every active session must bind the observed network public key into its
  handshake transcript.
- Dev-mode private key files are local-only and owner-readable. Production
  storage should move root recovery material into the OS keychain or an
  encrypted export before cross-device trust is advertised.

## Recovery Export Placeholder

`devrelay identity recovery-export` currently reports that export is unavailable.
The future export format should be encrypted, include the fabric root recovery
material, and exclude per-device network private keys.
