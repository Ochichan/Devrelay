# DevRelay Foundation Notes

Last updated: 2026-06-23

This document records the implementation contract that everything above the
core must preserve. It is no longer only a Phase 0 note; M0-M6 work has added
agent, metadata, lease, data-plane, background, and advanced Git behavior.

## Product Contract Read From The Spec

DevRelay's core promise is verified continuation, not byte-for-byte folder
sync. The implementation starts from these invariants:

- Snapshots are immutable once created.
- Canonical writer ownership is modeled separately from stored snapshots.
- Stale or inactive work must be preserved as a fork, not merged silently.
- Target workspaces are never overwritten while dirty.
- Secret-like files are excluded unless an explicit encrypted secret channel
  owns them.
- Git state is represented semantically: HEAD, index tree, working tree overlay,
  selected untracked paths, sidecars, operation capsules, and verification
  hashes.
- Watcher events are hints, not truth.
- UI state belongs to the local agent.

## Core Build Line

The foundation now consists of these layers:

1. Git state capture and verification.
2. Local CLI and per-project storage.
3. Agent JSON-RPC and event stream.
4. SQLite metadata and writer leases.
5. Pairing, transport-security primitives, revocation, and audit.
6. Git object data plane plus CAS sidecars.
7. Background protection and retention.
8. Cross-platform and advanced Git-state safety.

The desktop UI should sit above these layers and consume the agent boundary,
not reimplement them.

## Git Snapshot Semantics

Snapshot creation:

1. Parse and validate `devrelay.toml`.
2. Collect Git status through `git status --porcelain=v2 -z`.
3. Build two synthetic commits:
   - `I`: current index tree, parented by `HEAD`
   - `W`: current working tree plus accepted untracked files, parented by `I`
4. Store snapshot refs under `refs/devrelay/snapshots/<id>/`.
5. Persist metadata in the per-project SQLite store.
6. Store synthetic objects in the per-project bare repo where feasible.

Snapshot apply:

1. Refuse or preserve dirty target work according to the dirty policy.
2. Fetch required snapshot refs and sidecars.
3. Reset target to source `HEAD`.
4. Materialize `W` into the working tree.
5. Reset only the index to `I`.
6. Verify HEAD, index tree, work tree, sidecars, included untracked paths, and
   state hash.

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

`apply` refuses dirty targets by default. `snapshot-and-fork` captures a pinned
dirty-target backup before applying, while `new-workspace` leaves the dirty
target unchanged and applies into a sibling workspace.

## Agent And UI Boundary

The agent owns user-visible DevRelay state:

- project registry
- workspace state
- latest checkpoint/protection status
- lease holder
- handoff progress
- dirty target safe actions
- environment readiness
- diagnostics and activity events

Production UI must consume initial RPC state and the event stream. It must not
read Git directly or compute canonical state from filesystem observations.

## Windows And WSL Workspace Boundaries

DevRelay treats Windows native and WSL as separate device boundaries, even when
they run on the same physical machine. Each WSL distro is also a separate
boundary. WSL-owned checkouts should live on the distro filesystem, such as
`/home/<user>`, rather than under `/mnt/c`. Windows-native tools should use a
separate clone instead of mutating a WSL tree through `\\wsl$`.

Use `devrelay doctor wsl-filesystem --repo <path>` to flag shared-tree mutation
risk and print the current workspace mapping guidance.

Windows production UI is blocked until Windows named pipe IPC and per-user pipe
ACL are complete.

## Core API Boundary

`devrelay-core` exposes the workspace-internal API from the crate root. The
reviewed surface is recorded in [api-surface.md](api-surface.md). The stable
product contracts are CLI JSON, agent RPC/event schemas, snapshot schema, and
safety policies rather than private module structure.

## Verification Coverage

The base gate is covered by:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `crates/devrelay-core/tests/round_trip.rs` for staged, unstaged, untracked,
  secret exclusion, binary, executable-bit, rename, dirty-target, source
  unchanged, and target status equivalence fixtures
- `crates/devrelay-cli/tests/cli.rs` for CLI flows, registry, recovery, dirty
  policy, and local continue behavior
- `crates/devrelay-agent/tests/agent.rs` for agent smoke and RPC behavior

Product safety still needs named integration suites listed in
[data-loss-safety.md](data-loss-safety.md).
