# Contributing

DevRelay has moved from local correctness into agent, lease, data-plane, and
pre-UI product work. Keep changes small, testable, and tied to
`docs/north-star-checklist.md`.

## Local Setup

Install the pinned Rust toolchain and `just`:

```bash
rustup toolchain install 1.96.0
cargo install just
```

Run the local preflight before opening a pull request:

```bash
just preflight
```

Useful individual checks:

```bash
just fmt-check
just clippy
just test
```

## Branches

Use the branch naming convention in `docs/branching.md`.

## Change Discipline

- Update `docs/north-star-checklist.md` when a checklist item is complete and
  verified.
- Update `docs/current-state.md` when a milestone status changes.
- Update `docs/api-surface.md` before changing CLI JSON, RPC, event, snapshot,
  or UI authority boundaries.
- Add or update tests with behavior changes.
- Keep Git state, data-loss safety, and recovery behavior explicit in PRs.
- Do not mark milestone exit gates complete until every gate item is complete.
- Do not mark safety invariants complete until they are tied to a named suite in
  `docs/data-loss-safety.md`.
- Do not claim invisible background protection without resource evidence from
  `docs/resource-benchmark.md`.
