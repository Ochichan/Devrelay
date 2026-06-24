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
