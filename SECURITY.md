# Security Policy

DevRelay handles local source code, Git state, environment intent, and eventually
device trust. Treat data-loss and privacy bugs as security-sensitive until the
project has a richer classification process.

## Reporting

Do not file public issues for suspected credential exposure, secret handling
failures, device identity bugs, or data-loss paths. Send a private report to the
maintainers with:

- the affected command or workflow
- the observed behavior
- reproduction steps
- whether local work, secrets, or device trust could be affected

## Current Scope

The current implementation includes a Rust core, CLI, local agent, SQLite
metadata, recovery, lease state, pairing and mTLS primitives, revocation, audit
logs, Git object data-plane work, CAS sidecars, and background protection
pieces.

The following are not production security boundaries yet:

- M4.5 remote Control API
- Windows named pipe IPC and pipe ACL
- production desktop UI
- encrypted secret sync or secret provider materialization
- signed release/update channel
- independent security review

## Baseline Expectations

- Dirty target workspaces must not be overwritten silently.
- Secret-like files must remain excluded unless an explicit encrypted secret
  channel owns them.
- Diagnostics and future logs must avoid raw secrets and private paths where
  possible.
- Revoked devices must not publish, connect, or transfer leases.
- Stale and inactive work must be preserved without advancing canonical latest.
- Handoff must not transfer writer authority before target verification.
- UI must not compute canonical state outside the local agent.
