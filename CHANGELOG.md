# Changelog

All notable DevRelay changes should be recorded here.

This project uses milestone-oriented entries while the public release process is
being established.

## Unreleased

- Added remote access credential distribution and a remote CLI: `devrelay
  remote credentials issue` packages the fabric TLS CA, peer TLS leaf, and
  device certificate from a confirmed pairing session, `devrelay remote
  credentials import` validates and stores the bundle on the peer, and
  `devrelay remote call` drives the mTLS Control RPC API end to end, covered
  by a CLI integration test.
- Completed the named non-negotiable safety suites
  (environment_failure_leaves_code_intact, ui_has_no_state_authority) and
  recorded representative macOS resource benchmark evidence with numeric
  idle/burst budgets.
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
