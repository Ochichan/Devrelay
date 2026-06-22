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

The current implementation is a local Rust CLI/core foundation. It does not yet
include pairing, network transport, encrypted secret sync, or a desktop agent.

## Baseline Expectations

- Dirty target workspaces must not be overwritten silently.
- Secret-like files must remain excluded unless an explicit encrypted secret
  channel owns them.
- Diagnostics and future logs must avoid raw secrets and private paths where
  possible.

