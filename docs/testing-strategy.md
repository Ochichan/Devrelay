# Testing Strategy

Last updated: 2026-06-23

DevRelay tests must prove behavior at the same boundary users trust. Unit tests
are necessary, but product safety gates require integration evidence.

## Test Layers

### Unit Tests

Use for deterministic pure logic:

- manifest parsing and validation
- Git porcelain parsing
- path classification
- state hashing
- lease transition validation
- route selection scoring
- retention planning
- trust hash calculation
- RPC envelope parsing

### Integration Tests

Use for behavior that crosses storage, Git, IPC, or process boundaries:

- snapshot create/apply/verify
- dirty target refusal and backup
- local project registry and SQLite migrations
- recovery open
- CLI through agent
- event stream reconnect
- handoff state machine and crash recovery
- CAS materialization
- data-plane authorization

### Product Safety Suites

Use named suites for non-negotiable invariants:

- `safety/no_silent_overwrite`
- `safety/no_unverified_handoff`
- `safety/stale_publish_is_fork`
- `safety/no_plaintext_secret_snapshot`
- `safety/ui_has_no_state_authority`
- `safety/watcher_events_are_hints`
- `safety/no_active_workspace_remote_task`

Each suite should include a short README or module doc that names the invariant
and links back to [data-loss-safety.md](data-loss-safety.md).

## Fuzzing

Fuzz trust-boundary parsers and payloads:

- `manifest_parser`
- `porcelain_parser`
- `path_canonicalization`
- `cas_manifest`
- `network_api_payload`

See [fuzzing.md](fuzzing.md).

## Fault Injection

Fault injection should cover every phase that can leave durable state:

- snapshot object write
- ref update
- metadata transaction
- CAS upload
- publish
- target fetch
- base/work/index apply
- verification
- lease commit

The expected result is zero data loss in supported states, not merely a clean
error.

## Resource Benchmarks

Resource measurements are release evidence and must be repeatable. See
[resource-benchmark.md](resource-benchmark.md).

## UI Verification

Before the first UI slice is considered complete:

- UI state must come from initial agent RPC plus event subscription.
- Dirty target flow must avoid Git jargon.
- Critical state must not rely only on color.
- Keyboard-only handoff must work.
- Primary controls must have accessible names.
- Real-device macOS-to-Linux dogfood must be recorded.

Manual runtime checks for the current desktop shell live in
[manual-runtime-checklist.md](manual-runtime-checklist.md). Run them before
claiming a build is ready for dogfood.

`npm run check:ui --prefix apps/desktop` also runs a lightweight frontend VM
smoke test for `devrelay-agent-connected`, `devrelay-agent-event`,
`devrelay-agent-gap`, and `devrelay-agent-disconnected`. This guards the
desktop event bridge state, stale-data indicator, and bootstrap refresh path
without launching Tauri.

## Local Commands

```bash
just fmt-check
just clippy
just test
just preflight
```

Supply-chain and inventory checks:

```bash
just audit
just dependency-inventory
just tooling
```
