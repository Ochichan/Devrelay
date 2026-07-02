# DevRelay

DevRelay is a personal development fabric. It does not sync mutable folders.
It preserves and transfers verified development sessions: Git HEAD, index,
working tree changes, selected untracked files, environment intent, and writer
ownership.

The repository has moved past the original local round-trip proof. The current
core includes the Rust CLI/core, local agent RPC, SQLite metadata, recovery,
single-writer lease state, pairing and mTLS primitives, revocation, audit logs,
the remote Control RPC server over mTLS with fabric-issued device
certificates, per-project bare Git object storage, sidecar CAS, route
selection, retention, cross-platform doctors, and advanced Git-state capture. The desktop Tauri shell
now includes the tray/dashboard first slice: agent-backed runtime, project
status, checkpoint, source-side handoff prepare/abort, target-side
apply/verify/commit, diagnostics, settings, accessibility, and overflow-safe
screens. Real macOS/Linux cross-device dogfood is still the next product cut.

## Current Product Cut

The next product cut is intentionally narrow:

```text
Mac project -> Linux workstation -> two-click verified continuation
```

That first vertical slice must prove:

- the local agent is the only authority for UI state
- current work is protected before handoff
- the writer device is obvious
- the target device readiness is visible
- dirty target work is preserved instead of overwritten
- handoff completes only after verification

Windows remains a cross-platform hardening target, but the first UI dogfood can
ship as macOS/Linux-only unless Windows named pipe IPC and pipe ACLs are
finished first.

## Quick Start

```bash
just preflight
cargo run -p devrelay-cli -- --version
cargo run -p devrelay-cli -- manifest check devrelay_spec_bundle/devrelay.toml
cargo run -p devrelay-cli -- manifest check devrelay_spec_bundle/devrelay.toml --json
cargo run -p devrelay-cli -- project add . --manifest devrelay_spec_bundle/devrelay.toml
cargo run -p devrelay-cli -- status --repo . --manifest devrelay_spec_bundle/devrelay.toml
```

Individual local checks are available through `just fmt-check`, `just clippy`,
and `just test`. Supply-chain checks are available through `just audit`,
`just dependency-inventory`, and `just tooling`.

`status`, `checkpoint`, and `continue` must be run against actual Git
repositories. This project directory is only the DevRelay implementation
workspace unless you intentionally use it as a registered test project.

## Important Docs

- [Current state](docs/current-state.md)
- [North Star roadmap](docs/north-star-roadmap.md)
- [Execution checklist](docs/north-star-checklist.md)
- [First UI vertical slice](docs/ui-vertical-slice.md)
- [Desktop frontend guide](apps/desktop/AGENTS.md)
- [Manual verification runbook](docs/manual-verification-runbook.md)
- [Manual runtime checklist](docs/manual-runtime-checklist.md)
- [Dogfood scenarios](docs/dogfood-scenarios.md)
- [Install, update, and removal](docs/install-update.md)
- [API surface](docs/api-surface.md)
- [Remote Control RPC API](docs/remote-rpc-api.md)
- [CLI reference](docs/cli.md)
- [Data-loss safety policy](docs/data-loss-safety.md)
- [Resource benchmark plan](docs/resource-benchmark.md)
- [Testing strategy](docs/testing-strategy.md)

The bundled Korean North Star spec remains the product definition. The docs in
`docs/` are the live implementation plan and must be kept aligned with the
codebase.
