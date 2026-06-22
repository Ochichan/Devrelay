# DevRelay North Star Roadmap

Last updated: 2026-06-22

This roadmap turns the North Star spec into an execution plan. It assumes the
current repository starts with a Rust CLI/core foundation and a static UI
prototype, then grows toward a local-first personal development fabric.

The issue-sized execution checklist lives in
[`docs/north-star-checklist.md`](north-star-checklist.md).

## 0. Direction

DevRelay should not become a folder sync tool. The product succeeds when a user
can move a verified development session between personal machines without
thinking about push, stash, patch files, environment setup, or which clone is
authoritative.

The implementation order must preserve that identity:

1. Prove Git state capture and apply correctness.
2. Add explicit local CLI workflows.
3. Add anchor metadata and writer lease safety.
4. Add background protection only after explicit workflows are safe.
5. Add desktop/editor UX once the agent API is stable.
6. Add environment hydration.
7. Add compute fabric.
8. Harden advanced Git states, security, operations, and migration.

## 1. Research Anchors

The plan is grounded in the bundled North Star spec and these current technical
references:

- Git protocol v2 is capability-based and extensible, which fits the data-plane
  direction for Git object transfer.
  <https://git-scm.com/docs/protocol-v2>
- `git status --porcelain=v2 -z` is the right machine interface for status; the
  parser must ignore unknown headers.
  <https://git-scm.com/docs/git-status>
- `git write-tree` creates a tree from the current index and requires a fully
  merged index, so conflicted/rebase states need a separate operation capsule
  milestone.
  <https://git-scm.com/docs/git-write-tree>
- `git read-tree` is the plumbing command for loading tree data into the index
  and, with the right flags, the work tree.
  <https://git-scm.com/docs/git-read-tree>
- Git FSMonitor can reduce status work, but it is a performance option and must
  not be treated as the source of truth.
  <https://git-scm.com/docs/git-fsmonitor--daemon>
- SQLite WAL is appropriate for one active anchor with concurrent local readers,
  but it does not turn SQLite into distributed consensus.
  <https://sqlite.org/wal.html>
- mDNS is link-local discovery, which matches DevRelay's LAN-first pairing and
  peer discovery model.
  <https://datatracker.ietf.org/doc/html/rfc6762>
- TLS 1.3 and Ed25519/EdDSA are the right baseline primitives for authenticated
  transport and device/snapshot signatures.
  <https://datatracker.ietf.org/doc/html/rfc8446>
  <https://datatracker.ietf.org/doc/html/rfc8032>
- QUIC is a plausible later data/control transport, but HTTP/2 over TLS is a
  lower-risk first network implementation.
  <https://datatracker.ietf.org/doc/html/rfc9000>
- Tauri v2 supports a Rust backend with webview frontend across desktop
  platforms, matching the thin UI over local-agent RPC model.
  <https://v2.tauri.app/>
- VS Code extensions can use workspace APIs for filesystem/workspace events,
  but the extension should report context and status rather than manipulate Git.
  <https://code.visualstudio.com/api/references/vscode-api>
- Nix remote builds and binary caches should be used instead of building a
  parallel distributed build protocol for Nix workloads.
  <https://nix.dev/manual/nix/2.18/advanced-topics/distributed-builds>
  <https://nix.dev/guides/recipes/add-binary-cache.html>
- Service installation needs platform-native packaging: launchd/LaunchAgent on
  macOS, systemd user services on Linux, and per-user/background service
  strategy on Windows.
  <https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html>
  <https://www.freedesktop.org/software/systemd/man/systemd.service.html>
  <https://learn.microsoft.com/en-us/windows/application-management/per-user-services-in-windows>
  <https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes>

## 2. Current Baseline

Already started in this repository:

- Rust workspace with `devrelay-core` and `devrelay-cli`
- Manifest parser and validator for `devrelay.toml`
- Git status collection through the installed Git CLI
- Safe untracked classification and secret-name hard blocks
- Synthetic index/work snapshot metadata
- Local source-to-target snapshot apply and verification
- Unit tests for normal staged, unstaged, untracked, and dirty-target refusal

Immediate gap: this is still a local proof, not a product. There is no registry,
agent, anchor DB, lease, journal, pairing, network transfer, environment
hydration, task runner, desktop UI, or editor integration.

## 3. Execution Model

### Workstream A: Core Correctness

Owns Git state, snapshot semantics, apply verification, operation capsules,
sidecar CAS, path safety, and round-trip tests.

### Workstream B: Local Agent And Anchor

Owns long-running service, local IPC, SQLite metadata, lease state machine,
handoff orchestration, retention, journal, and recovery.

### Workstream C: Security And Trust

Owns device identity, pairing, mTLS, command trust hashes, snapshot signatures,
secret classification, revoke, audit, and diagnostic redaction.

### Workstream D: UX Surfaces

Owns CLI, tray/dashboard, command palette, actionable errors, recovery timeline,
and editor context adapters.

### Workstream E: Environment And Compute

Owns Nix/devcontainer/native profiles, secret providers, prewarm, scheduler,
remote task runner, logs, artifacts, and result cache.

## 4. Milestone Map

### M0. Foundation: Local Git Round Trip

Status: partially implemented.

Goal: prove the central technical claim: supported Git states can be captured,
applied to another clone, and verified without changing user history.

Build:

- Keep the current Rust CLI/core structure.
- Harden porcelain v2 parser:
  - branch headers
  - ordinary entries
  - rename entries
  - untracked entries
  - ignored entries
  - unknown header tolerance
- Replace ad hoc snapshot IDs with schema-versioned IDs.
- Separate metadata schema from runtime structs.
- Add target verification details:
  - HEAD OID
  - index tree OID
  - work tree OID
  - included untracked content root
  - excluded path reasons
- Add apply journal skeleton even for local apply.
- Add `devrelay status --json`, `checkpoint --json`, `apply --json`.
- Add test fixtures for:
  - staged add/modify/delete
  - unstaged add/modify/delete
  - untracked include/exclude
  - executable bit on POSIX
  - binary files
  - Unicode paths
  - secret filename blocks
  - dirty target refusal

Exit gate:

- `cargo test`, `cargo clippy -D warnings`, and round-trip fixture tests pass.
- Source repository status is unchanged after checkpoint.
- Target repository status is semantically equivalent after apply.
- Dirty target is blocked with a stable error code.

Do not build yet:

- background watchers
- network transfer
- GUI
- conflict/rebase support

### M1. Local CLI MVP

Goal: a single user can register projects and explicitly checkpoint, list,
recover, and apply sessions on one machine or local paths.

Build:

- Local data directory layout:
  - `$DEVRELAY_HOME/config.toml`
  - `$DEVRELAY_HOME/projects/<project-id>/metadata.sqlite`
  - `$DEVRELAY_HOME/projects/<project-id>/snapshots.git`
  - `$DEVRELAY_HOME/projects/<project-id>/cas/`
- Project registry:
  - `devrelay project add <path>`
  - `devrelay projects list`
  - remote URL/root commit fingerprint
  - workspace path mapping
- Snapshot store:
  - move synthetic objects into side snapshot repo where feasible
  - keep main repo refs minimal
  - snapshot metadata persisted in SQLite
- Recovery:
  - `devrelay recover list`
  - `devrelay recover show <snapshot-id>`
  - `devrelay recover open <snapshot-id> --path <new-workspace>`
- Local handoff simulator:
  - source path and target path on same machine
  - dirty target backup snapshot
  - apply journal resume/rollback
- Stable error schema:
  - code
  - title
  - detail
  - safe actions
  - diagnostic ID

Exit gate:

- A user can run all core CLI flows without editing config manually.
- Local apply never overwrites dirty target data.
- Recovery always opens a new workspace or clearly refuses.
- SQLite schema migrations have tests.

### M2. Agent And Local IPC

Goal: convert CLI-only logic into a user-session agent that owns state and
exposes stable local APIs to CLI, UI, and future editor plugins.

Build:

- `devrelay-agent` binary.
- Local IPC:
  - Unix domain socket on macOS/Linux
  - Windows named pipe
  - versioned JSON-RPC first; protobuf can come later
- Agent API:
  - status
  - project registry
  - checkpoint
  - recover
  - apply
  - diagnostics
- Event stream:
  - `workspace.state.changed`
  - `snapshot.local.created`
  - `snapshot.apply.started`
  - `snapshot.apply.verified`
  - `security.blocked`
  - `quota.warning`
- Service install dev mode:
  - macOS LaunchAgent template
  - Linux systemd user service template
  - Windows per-user/background process strategy
- Structured logs:
  - JSON lines
  - redaction
  - log rotation

Exit gate:

- CLI can operate entirely through the agent.
- Agent restart preserves registry and latest snapshot state.
- IPC access is local-user scoped.
- Diagnostic bundle excludes source code by default.

### M3. Anchor Metadata And Single-Writer Lease

Goal: introduce the control-plane concept that makes DevRelay distinct from
Git sync: one canonical writer and safe forks for stale/inactive work.

Build:

- Anchor mode in `devrelay-agent`.
- SQLite WAL metadata DB:
  - devices
  - projects
  - workspaces
  - sessions
  - snapshots
  - leases
  - handoffs
  - task_runs placeholder
- Lease state machine:
  - active
  - handoff_pending
  - committing
  - inactive
  - forked
  - archived
- CAS-safe canonical update:
  - compare `session_id`, `epoch`, `holder_device_id`, `state`
  - stale epoch publish cannot change latest pointer
- Manual device identity:
  - device ID
  - device display name
  - local certificate/key placeholder
- Explicit handoff over local/LAN manually configured addresses:
  - begin
  - prepare target
  - target verified
  - source generation check
  - commit lease
  - abort
- Inactive edit policy:
  - detect publish from stale holder
  - preserve as fork
  - never update canonical latest

Exit gate:

- Stale lease publish tests cannot advance canonical latest.
- Concurrent handoff attempts resolve deterministically.
- Target dirty protection happens before lease transfer.
- Crash between apply and lease commit is recoverable.

### M4. LAN Pairing And Secure Control Plane

Goal: make multiple devices usable without manual trust shortcuts.

Build:

- Fabric identity:
  - root key creation
  - device key creation
  - recovery export placeholder
- Pairing:
  - short authentication string from handshake transcript
  - user confirmation
  - device certificate issuance
- mDNS/DNS-SD discovery:
  - `_devrelay-anchor._tcp.local`
  - `_devrelay-peer._tcp.local`
  - only protocol/fabric hint/device ID/port in TXT
- mTLS transport:
  - rustls-based TLS 1.3
  - certificate pinning
  - revocation denylist
  - request nonce/timestamp
- Control API `/v1`:
  - devices
  - projects
  - workspaces
  - snapshots
  - handoffs
  - recovery
- Security audit log:
  - pair
  - revoke
  - lease transfer
  - command approval
  - security block

Exit gate:

- Pairing resists passive LAN observers and obvious MITM without matching code.
- Revoked devices cannot connect or publish.
- mDNS does not leak project names, paths, or user names.
- Transport tests cover expired, wrong-fabric, and revoked certificates.

### M5. Data Plane: Git Object Transfer And Sidecar CAS

Goal: stop relying on local filesystem paths and support direct/anchor object
transfer for real devices.

Build:

- Git object data plane:
  - first implementation may tunnel Git CLI over mTLS or use local bare repo
    fetch/push endpoints
  - restrict refs to `refs/devrelay/*`
  - quota and object size checks
- Anchor snapshot bare repos.
- Direct peer route:
  - source online: fetch from source
  - source offline: fetch from anchor cache
- CAS:
  - chunk hash
  - missing query
  - upload/download
  - manifest
  - reachability mark/sweep
- Large untracked/ignored handling:
  - threshold into CAS
  - bounded-memory streaming
  - sidecar root hash verification
- Apply materialization:
  - staged area fetch
  - sidecar materialize
  - hash verification

Exit gate:

- Large sidecar transfer uses bounded memory.
- Missing/corrupt chunk blocks apply before active handoff.
- Anchor cache can serve a snapshot after source is offline.
- Partial upload never changes canonical latest.

### M6. Background Protection

Goal: make DevRelay quietly protective without becoming noisy or resource-heavy.

Build:

- Filesystem watcher as hint only:
  - macOS FSEvents
  - Linux inotify/fanotify abstraction
  - Windows ReadDirectoryChangesW
- Adaptive debounce:
  - quiet window
  - max dirty interval
  - publish quiet window
- Git-aware scan:
  - porcelain v2 status
  - optional FSMonitor doctor
  - optional untracked cache doctor
- Resource policy:
  - adaptive/instant/eco/custom
  - CPU slots
  - bandwidth cap
  - battery/low-power behavior
- Retention:
  - recent dense checkpoints
  - hourly/daily thinning
  - pinned snapshots
  - handoff/operation pins
- Quota:
  - device quota
  - anchor project quota
  - free disk stop points
- Crash journal:
  - snapshot creation
  - publish
  - target apply
  - lease transition

Exit gate:

- Idle agent CPU/RSS meets budget on representative repos.
- Formatter touching many files coalesces into bounded snapshot work.
- Disk pressure prunes only unpinned/evictable data.
- Background failures surface as protection status, not repeated notifications.

### M7. Desktop UX: Tray And Dashboard

Goal: ship the day-to-day product surface around "Continue here" and "Run
elsewhere" without exposing Git plumbing by default.

Build:

- Tauri desktop shell:
  - sidebar dashboard
  - tray/menu bar
  - command palette
  - native notifications
  - reduced-motion/accessibility pass
- Views:
  - Continue
  - Projects
  - Devices
  - Runs
  - Activity
  - Settings
- Handoff dialog:
  - source
  - target
  - checkpoint age
  - staged/unstaged/untracked summary
  - environment warmth
  - dirty target safe actions
- Error UX:
  - stable code
  - human title
  - safe actions
  - explain link
- Recovery timeline:
  - checkpoint list
  - pins
  - open as new session
- Advanced diagnostics:
  - snapshot IDs
  - lease epoch
  - transfer route
  - verification hashes

Exit gate:

- User can complete same-LAN handoff from tray in two clicks.
- No critical state is communicated by color alone.
- Dirty target flow is understandable without Git terminology.
- UI state comes from agent events, not duplicated frontend logic.

### M8. Editor Context: VS Code First

Goal: restore human context after handoff without pretending process state can
move across operating systems.

Build:

- VS Code extension:
  - local agent connection
  - active/inactive indicator
  - opened files
  - tab order
  - active editor
  - cursor/selection
  - split layout
  - breakpoints where supported
  - terminal cwd/title layout
- Unsaved buffer capsule:
  - explicit encrypted context overlay
  - restored as dirty buffer
  - never silently saved
- Handoff edit guard:
  - source generation increment on editor changes
  - handoff abort if source changes before commit
- Extension command palette entries:
  - continue here
  - checkpoint
  - run task
  - open recovery

Exit gate:

- Handoff opens target editor with correct files/cursors.
- Unsaved buffers restore as unsaved/dirty, not disk writes.
- Extension failure does not block verified code handoff.
- Source edit during handoff prevents stale lease transfer.

### M9. Environment Hydration

Goal: make "code ready" and "environment ready" explicit, reproducible, and
safe.

Build:

- Manifest trust hashes:
  - environment command
  - healthcheck
  - task command
  - bootstrap script
- Profile selection:
  - Nix
  - Dev Container
  - native script
  - manual
- Nix adapter:
  - `nix develop`
  - flake fingerprint
  - platform-specific cache warmth
  - optional LAN binary cache
- Native bootstrap adapter:
  - PowerShell on Windows
  - POSIX scripts only with trust
  - idempotency guidance
- Secret providers:
  - OS keychain first
  - 1Password CLI
  - Bitwarden CLI
  - SOPS/age
  - user script provider
- Hydration states:
  - cold
  - metadata-ready
  - cache-ready
  - shell-ready
  - app-ready
- Doctor:
  - missing toolchain
  - command hash changed
  - secret missing
  - portability issue

Exit gate:

- Manifest command changes require explicit trust.
- Required secrets are materialized locally but excluded from snapshots.
- Environment failure leaves code state intact and actionable.
- Warm target can enter dev shell within the SLO budget for representative
  projects.

### M10. Compute Fabric

Goal: use idle personal machines for builds, tests, benchmarks, and agent work
without taking writer ownership.

Build:

- Task model:
  - immutable execution snapshot
  - isolated task workspace
  - environment hydrate
  - command run
  - logs/artifacts/result
- Scheduler:
  - hard constraints
  - score model
  - cache warmth
  - idle CPU
  - memory/disk
  - AC/battery
  - network/transfer cost
  - foreground penalty
- Runner isolation:
  - host
  - sandbox
  - container
  - VM placeholder
- Logs:
  - bounded live stream
  - disk spool
  - redaction
- Artifacts:
  - declared output capture
  - hash verification
  - summary first
  - on-demand pull for large artifacts
- Nix delegation:
  - generate temporary builder set
  - integrate remote builder logs
  - publish to LAN binary cache
- Code-changing agent task:
  - separate session
  - commit chain or verified diff
  - never auto-merge into active session

Exit gate:

- Remote task cannot mutate canonical active session.
- Scheduler explains why it chose a target.
- Cancellation kills process trees.
- Artifact capture cannot escape declared output policy.

### M11. Cross-Platform Hardening

Goal: make macOS, Linux, Windows native, and WSL behavior predictable.

Build:

- Platform identity:
  - OS
  - arch
  - ABI
  - WSL distro/version
- Path portability doctor:
  - case collisions
  - Unicode normalization collisions
  - reserved Windows names
  - trailing dot/space
  - invalid Windows characters
  - path length budget
  - symlink capability
- Line ending policy:
  - `.gitattributes` source of truth
  - detect `core.autocrlf` conflicts
- Executable bit verification.
- Symlink policy:
  - preserve link target string
  - never follow outside workspace
- Windows reparse point defense.
- WSL/native split:
  - independent Device identity
  - independent Workspace mapping
  - no shared tree mutation between Windows and WSL

Exit gate:

- Case-sensitive to case-insensitive target blocks unsafe paths pre-apply.
- Windows native and WSL are represented as separate devices.
- Symlink/reparse path traversal tests pass.
- Line-ending differences verify by Git semantics, not raw byte equality.

### M12. Advanced Git States

Goal: expand beyond normal index without compromising correctness.

Build order:

1. Merge/cherry-pick/revert conflicts:
   - stage 1/2/3 index manifest
   - control metadata
   - round-trip verification
2. Submodules:
   - clean submodule commit restore
   - dirty submodule as child project/session
   - recursion limits
3. Git LFS:
   - pointer file in Git snapshot
   - local-only LFS object fallback into CAS
   - missing object block
4. Sparse checkout/partial clone:
   - distinguish logical project state from local sparse view
   - target sparse policy not overwritten
5. Interactive rebase/sequencer:
   - internal operation capsule
   - target Git version compatibility
   - fallback to block and offer safe alternatives

Exit gate:

- Conflict round-trip preserves staged entries and user can continue resolving.
- Rebase support is disabled until operation capsule tests are exhaustive.
- Unsupported operation states are blocked with recovery options.

### M13. Security, Privacy, And Operations Gate

Goal: get the system ready for real daily use on private machines.

Build:

- Threat model document:
  - LAN attacker
  - malicious manifest
  - stale device
  - compromised device
  - path traversal
  - secret leakage
  - replay
- Fuzzing:
  - porcelain parser
  - manifest parser
  - path canonicalization
  - CAS manifest/chunk metadata
  - network API payloads
- Secret scanning:
  - filename rules
  - private key headers
  - token patterns
  - high entropy heuristic
  - redacted logs
- Update strategy:
  - signed releases
  - migration compatibility
  - rollback expectations
- Backup anchor:
  - async metadata/snapshot copy
  - signed state
  - manual promotion
- Opaque anchor research:
  - encrypted bundles
  - reduced dedup tradeoff
  - opt-in mode
- Diagnostic bundle:
  - redacted config
  - recent logs
  - state machine records
  - hash/timing
  - no source code by default

Exit gate:

- Independent security review has no unresolved critical/high findings.
- Revocation blocks new access and lease operations.
- Diagnostic export has snapshot/source exclusion tests.
- Fault injection produces zero data loss in supported states.

### M14. Beta Product Loop

Goal: make the product reliable for real daily development before expanding
scope.

Build:

- Installer/dev channel for macOS, Linux, Windows, WSL.
- Onboarding:
  - first device
  - anchor selection
  - pair device
  - project discovery
  - environment trust
- Guided doctor:
  - project safety
  - path portability
  - environment
  - secret mapping
  - resource policy
- Product metrics local-only:
  - verified continuation rate
  - checkpoint success
  - apply verification failure
  - handoff phase durations
  - environment hydrate duration
  - scheduler choice reason
- User-facing docs:
  - quick start
  - recovery
  - unsupported states
  - security model
  - backup

Exit gate:

- One-click handoff works for normal projects between two real devices.
- Target dirty and inactive edit flows are safe and understandable.
- No Git command is required for normal handoff.
- Local-only telemetry/metrics remain private by default.

### M15. North Star Release Candidate

Goal: meet the final product definition from the spec.

Required capabilities:

- Anchor-first topology.
- Manual/static discovery fallback.
- Pair/revoke devices.
- Project registry across devices.
- Single canonical writer lease.
- Explicit and background checkpoints.
- Direct/anchor snapshot transfer.
- Dirty target recovery and inactive fork preservation.
- Timeline recovery as new session.
- Tauri tray/dashboard.
- VS Code context restore.
- Nix environment hydration and health checks.
- Secret reference materialization.
- Remote task runner with scheduler and artifacts.
- Cross-platform portability doctor.
- Diagnostic bundle and audit log.

North Star gates:

- Verified continuation:
  - prefetched target p95 under 5 seconds to code-ready
  - warm environment p95 under 15 seconds to shell-ready
  - supported Git state fidelity 100 percent
  - data loss tolerance 0
- Correctness:
  - randomized round-trip suite at high volume
  - fault injection at every apply/publish/lease phase
  - stale lease canonical update count 0
  - secret hard-block regressions 0
- Resource:
  - idle CPU/RSS budget met
  - background network cap respected
  - battery policy respected
  - quota and GC protect pinned/latest data
- UX:
  - new user can pair second device without docs
  - target dirty state resolved safely
  - errors show safe actions
  - keyboard and screen reader pass
- Security:
  - mTLS mandatory
  - signed snapshot metadata
  - command trust hash
  - revocation
  - redacted diagnostics

## 5. Suggested Sequencing

The safest sequencing is not calendar-first. Each phase unlocks the next only
after its exit gate is met.

Suggested order for a small team:

1. M0-M1: 2-4 weeks
2. M2-M3: 4-6 weeks
3. M4-M5: 6-8 weeks
4. M6: 3-5 weeks
5. M7-M8: 5-8 weeks
6. M9: 4-6 weeks
7. M10: 5-8 weeks
8. M11-M13: continuous, with focused hardening waves
9. M14-M15: 6-10 weeks after real-device dogfooding starts

For a solo developer, expect the same sequence but a longer calendar. The
critical rule is to avoid building the polished desktop experience before the
lease, dirty-target, apply journal, and verification paths are boringly correct.

## 6. Immediate Next 10 Tasks

1. Add stable error codes and JSON error output to the existing CLI.
2. Add `SnapshotMetadata` schema tests and golden JSON fixtures.
3. Expand Git round-trip fixtures for staged delete, unstaged delete, binary,
   executable bit, and Unicode paths.
4. Add apply journal records to local apply.
5. Create SQLite schema and migrations for local project/session/snapshot data.
6. Add `devrelay project add/list` and local registry config.
7. Move snapshot metadata persistence into the registry.
8. Add recovery timeline CLI.
9. Add dirty target backup snapshot rather than only refusal.
10. Add a minimal agent process and make CLI call it in dev mode.

## 7. Non-Negotiables

- No silent overwrite.
- No automatic merge in background.
- No plaintext secret snapshot by default.
- No remote command execution without trust hash approval.
- No UI state that bypasses the agent's source of truth.
- No background watcher treated as truth.
- No cross-device handoff success until verification passes.
- No compute task writes directly into the active session.
