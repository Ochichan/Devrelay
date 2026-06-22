# Contributing

DevRelay is still in the local correctness phase. Keep changes small, testable,
and tied to `docs/north-star-checklist.md`.

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
- Add or update tests with behavior changes.
- Keep Git state, data-loss safety, and recovery behavior explicit in PRs.
- Do not mark milestone exit gates complete until every gate item is complete.

