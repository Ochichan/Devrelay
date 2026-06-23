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
devrelay manifest check <path> --json
devrelay status --repo <path> --manifest <path> [--json]
devrelay checkpoint --repo <path> --manifest <path> [--label <label>] [--pin] [--out <snapshot.json>]
devrelay snapshot list --project <project-id> [--json]
devrelay snapshot show <snapshot-id> --project <project-id> [--json]
devrelay snapshot export <snapshot-id> --project <project-id> --out <snapshot.json> [--json]
devrelay recover list [--project <project-id>] [--json]
devrelay recover show <snapshot-id> [--project <project-id>] [--json]
devrelay recover open <snapshot-id> --path <new-workspace> [--register] [--name <name>] [--json]
devrelay continue --source <source> --target <target> [--dirty-policy <policy>] [--dry-run] [--json]
devrelay apply --repo <target> --source <source> --snapshot <snapshot.json> [--dirty-policy <policy>] [--dry-run] [--json]
```

`apply` refuses dirty targets by default. That matches the product promise that
DevRelay never quietly overwrites local work.

`checkpoint` stores snapshot refs in the per-project bare repo under
`DEVRELAY_HOME` and persists queryable metadata in SQLite. Use `--out` or
`snapshot export` when a standalone snapshot metadata JSON file is needed.
`recover open` creates or reuses a clean recovery workspace, applies the selected
snapshot from the local store, and can register the recovered workspace.
`apply --dirty-policy block` is the default. `snapshot-and-fork` captures a
pinned dirty-target backup before applying, while `new-workspace` leaves the
dirty target unchanged and applies into a sibling workspace.
`continue` uses the same dirty policies for a local source-to-target handoff and
updates local workspace state placeholders when the workspaces are registered.
`apply --dry-run` validates that the target is clean and the source snapshot refs
are available without mutating the target.

## Windows And WSL Workspace Boundaries

DevRelay treats Windows native and WSL as separate device boundaries, even when
they run on the same physical machine. Each WSL distro is also a separate
boundary. WSL-owned checkouts should live on the distro filesystem, such as
`/home/<user>`, rather than under `/mnt/c`. Windows-native tools should use a
separate clone instead of mutating a WSL tree through `\\wsl$`.

Use `devrelay doctor wsl-filesystem --repo <path>` to flag shared-tree mutation
risk and print the current workspace mapping guidance.

## Core API Boundary

`devrelay-core` exposes the M0 API from the crate root. The manifest schema is a
public module because it is part of the project contract; Git orchestration,
policy classification, snapshot implementation, and error internals stay behind
private modules. The reviewed surface is recorded in
[`docs/api-surface.md`](api-surface.md).

## M0 Verification Coverage

The M0 gate is covered by:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `crates/devrelay-core/tests/round_trip.rs` for staged, unstaged, untracked,
  secret exclusion, binary, executable-bit, rename, dirty-target, source
  unchanged, and target status equivalence fixtures

Snapshot verification now returns structured details for HEAD, index tree, work
tree, state hash, included untracked paths, and excluded paths.
