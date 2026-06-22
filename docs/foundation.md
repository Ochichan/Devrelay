# DevRelay Foundation Notes

## Product Contract Read From The Spec

DevRelay's core promise is verified continuation, not byte-for-byte folder
sync. The implementation starts from these invariants:

- Snapshots are immutable once created.
- Canonical writer ownership is modeled separately from stored snapshots.
- Stale or inactive work must be preserved as a fork, not merged silently.
- Target workspaces are never overwritten while dirty.
- Secret-like files are excluded unless a future encrypted secret channel
  explicitly handles them.
- Git state is represented semantically: HEAD, index tree, working tree overlay,
  selected untracked paths, and verification hashes.

## Phase 0 Build Line

The first foundation is a CLI and core library that can prove a Git state
round trip:

1. Parse and validate `devrelay.toml`.
2. Collect Git status through `git status --porcelain=v2 -z`.
3. Build two synthetic commits:
   - `I`: current index tree, parented by `HEAD`
   - `W`: current working tree plus accepted untracked files, parented by `I`
4. Store snapshot refs under `refs/devrelay/snapshots/<id>/`.
5. Apply the snapshot to a clean target:
   - reset target to source `HEAD`
   - materialize `W` into the working tree
   - reset only the index to `I`
6. Verify HEAD, index tree, work tree, included untracked paths, and state hash.

This is deliberately local and explicit. Anchor SQLite, background publishing,
leases, device pairing, and UI surfaces come after this correctness gate.

## Initial CLI Surface

```bash
devrelay manifest check <path>
devrelay status --repo <path> --manifest <path> [--json]
devrelay checkpoint --repo <path> --manifest <path> [--out <snapshot.json>]
devrelay apply --repo <target> --source <source> --snapshot <snapshot.json>
```

`apply` refuses dirty targets by default. That matches the product promise that
DevRelay never quietly overwrites local work.

## Core API Boundary

`devrelay-core` exposes the M0 API from the crate root. The manifest schema is a
public module because it is part of the project contract; Git orchestration,
policy classification, snapshot implementation, and error internals stay behind
private modules. The reviewed surface is recorded in
[`docs/api-surface.md`](api-surface.md).
