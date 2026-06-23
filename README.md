# DevRelay

DevRelay is a personal development fabric. It does not sync mutable folders.
It preserves and transfers verified development sessions: Git HEAD, index,
working tree changes, selected untracked files, environment intent, and writer
ownership.

This repository currently starts at the Phase 0 foundation from the bundled
North Star spec:

- typed `devrelay.toml` manifest loading and validation
- Git status collection through the installed Git CLI
- safe untracked-file classification with secret hard-blocks
- synthetic index/work snapshot metadata
- local source-to-target snapshot apply and verification
- a small `devrelay` CLI for `manifest check`, `status`, `checkpoint`, and
  `apply`

The UI prototype remains a product reference. The first implementation target
is correctness of state capture and round-trip behavior.

## Quick Start

```bash
just preflight
cargo run -p devrelay-cli -- --version
cargo run -p devrelay-cli -- manifest check devrelay_spec_bundle/devrelay.toml
cargo run -p devrelay-cli -- manifest check devrelay_spec_bundle/devrelay.toml --json
cargo run -p devrelay-cli -- status --repo . --manifest devrelay_spec_bundle/devrelay.toml
```

Individual local checks are available through `just fmt-check`, `just clippy`,
and `just test`. Supply-chain checks are available through `just audit`,
`just dependency-inventory`, and `just tooling`.

`status` and `checkpoint` must be run inside an actual Git repository. This
project directory is only the DevRelay implementation workspace unless you run
`git init`.

See [docs/cli.md](docs/cli.md) for CLI examples, JSON output, snapshot file
defaults, and exit code conventions.

## Current Scope

The first milestone intentionally excludes background watchers, LAN discovery,
SQLite anchor state, mTLS, desktop UI, and editor integration. Those belong
after the Git state round-trip gate is stable.

See [docs/foundation.md](docs/foundation.md) for the implementation contract,
[docs/north-star-roadmap.md](docs/north-star-roadmap.md) for the detailed path
to the North Star product, and
[docs/north-star-checklist.md](docs/north-star-checklist.md) for the full
execution checklist.
