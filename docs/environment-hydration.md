# Environment Hydration

Last updated: 2026-06-24

Environment hydration prepares the tools around a verified code handoff. It
must not rewrite project source state, hide command trust changes, or require a
successful editor restore before code can be verified.

## Native Bootstrap

Native bootstrap profiles run on the host through the command declared in
`devrelay.toml`. They are trust-gated before execution, run with the declared
timeout, capture redacted stdout/stderr, and report a healthcheck result.

Bootstrap commands must be idempotent. DevRelay may retry a failed hydration,
re-run a bootstrap after a target changes, or ask the user to approve a changed
command hash. A bootstrap script should be safe to run more than once, should
check whether tools or files already exist before mutating them, and should put
generated artifacts in ignored or explicitly managed locations.

## Secret Providers

Manifest secrets are mapped by local device configuration to provider
references. The current core model supports OS keychain design entries,
1Password CLI `op read`, Bitwarden CLI `bw get password`, SOPS/age decrypt,
and user-script command plans. Provider execution is still caller-owned, but
the materialization path is implemented and testable through a fake provider.

Secret files are written only inside the workspace, with restrictive file
permissions on Unix. Secret environment variables are returned as local
environment values for the caller to inject. Reports intended for logs redact
secret values. Manifest-declared secret file targets are hard-excluded from
snapshot untracked classification even when an include pattern matches them.
