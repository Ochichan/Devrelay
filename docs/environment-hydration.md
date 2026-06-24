# Environment Hydration

Last updated: 2026-06-24

Environment hydration prepares the tools around a verified code handoff. It
must not rewrite project source state, hide command trust changes, or require a
successful editor restore before code can be verified.

## Hydration State

The core hydration state machine records environment readiness separately from
Git handoff state. The normal path is `cold` -> `metadata-ready` ->
`cache-ready` -> `shell-ready` -> `app-ready`.

Any state can move to `failed` with a redacted failure summary. Only `failed`
can take the `retry` transition, which clears the failure, increments the
attempt counter, and returns the record to `cold`.

Hydration state can be saved and loaded as JSON through the core persistence
helpers. The agent or UI layer owns where that JSON record lives on disk.
Progress updates can be wrapped in the stable `environment.progress` event
payload so CLI, desktop, and editor clients can render the same state names.

## Environment Doctor

`devrelay doctor environment` diagnoses hydration blockers before the target
code state is changed. It reports missing Nix, missing Docker/Podman, missing
PowerShell, changed executable command hashes, missing required secret provider
mappings, incompatible profile targets, and opt-in healthcheck failures.

Each issue includes safe actions. Healthchecks are not run by default because
they can be expensive or trigger toolchain setup. Use `--run-healthcheck` when
the selected profile should be actively probed; Dev Container image preparation
still requires the separate `--allow-devcontainer-prepare` flag.

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
