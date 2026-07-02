# Changelog

All notable DevRelay changes should be recorded here.

This project uses milestone-oriented entries while the public release process is
being established.

## Unreleased

- Implemented the M4.5 remote Control RPC boundary: the agent serves the
  versioned JSON-RPC allowlist over mTLS behind `devrelay-agent
  --remote-listen`, authenticating every request against the pinned fabric
  root, revocation state, TLS channel key binding, replay nonces, and clock
  skew before dispatch, with security-blocked audit records for rejected
  requests, remote recovery methods, deterministic fabric X.509 certificate
  issuance, a minimal remote control client, and agent integration tests.
- Initialized repository hygiene, local preflight commands, CI templates, and
  architecture decision records.
- Added local agent, registry, recovery, lease, data-plane, CAS, background
  protection, cross-platform, and advanced Git-state documentation status.
- Realigned roadmap/checklist/API docs around the first macOS-to-Linux desktop
  continuation slice.
- Added safety, recovery, testing, resource benchmark, glossary, supported
  state, unsupported state, and UI vertical-slice docs.
