# DevRelay North Star Roadmap

Last updated: 2026-06-24

This roadmap turns the bundled North Star spec into a live execution plan. The
spec is the product target. This roadmap is the implementation path from the
current repository state to that target.

The issue-sized execution checklist lives in
[`docs/north-star-checklist.md`](north-star-checklist.md). The short status
source lives in [`docs/current-state.md`](current-state.md).

## 0. Direction

DevRelay should not become a folder sync tool. The product succeeds when a user
can move a verified development session between personal machines without
thinking about push, stash, patch files, environment setup, or which clone is
authoritative.

The four core decisions still hold:

1. Use a Rust core and orchestrate Git through the installed Git CLI.
2. Make the local agent the only authority for UI-visible state.
3. Do not build production GUI before core verification gates are credible.
4. Start Git object transfer from per-project bare repositories.

The repository now has enough core depth to begin a product UX slice. The next
priority is not more advanced core scope; it is making one real-device
continuation path safe, understandable, and repeatable.

## 1. Research Anchors

The plan is grounded in the bundled North Star spec and these technical
references:

- Git protocol v2 is capability-based and extensible, which fits the data-plane
  direction for Git object transfer. <https://git-scm.com/docs/protocol-v2>
- `git status --porcelain=v2 -z` is the right machine interface for status; the
  parser must ignore unknown headers. <https://git-scm.com/docs/git-status>
- `git write-tree` creates a tree from the current index and requires a fully
  merged index, so conflicted/rebase states need operation capsules.
  <https://git-scm.com/docs/git-write-tree>
- `git read-tree` is the plumbing command for loading tree data into the index
  and, with the right flags, the work tree.
  <https://git-scm.com/docs/git-read-tree>
- Git FSMonitor can reduce status work, but it is a performance option and must
  not be treated as the source of truth.
  <https://git-scm.com/docs/git-fsmonitor--daemon>
- SQLite WAL is appropriate for one active anchor with concurrent local
  readers, but it does not turn SQLite into distributed consensus.
  <https://sqlite.org/wal.html>
- mDNS is link-local discovery, matching DevRelay's LAN-first pairing and peer
  discovery model. <https://datatracker.ietf.org/doc/html/rfc6762>
- TLS 1.3 and Ed25519/EdDSA are the baseline primitives for authenticated
  transport and device/snapshot signatures.
  <https://datatracker.ietf.org/doc/html/rfc8446>
  <https://datatracker.ietf.org/doc/html/rfc8032>
- QUIC is a plausible later data/control transport, but HTTP/2 or RPC over TLS
  is lower-risk first. <https://datatracker.ietf.org/doc/html/rfc9000>
- Tauri v2 supports a Rust backend with webview frontend across desktop
  platforms, matching the thin UI over local-agent RPC model.
  <https://v2.tauri.app/>
- VS Code extensions can use workspace APIs for editor context, but extensions
  should report context and commands through the agent rather than manipulating
  Git. <https://code.visualstudio.com/api/references/vscode-api>
- Nix remote builds and binary caches should be used instead of building a
  parallel distributed build protocol for Nix workloads.
  <https://nix.dev/manual/nix/2.18/advanced-topics/distributed-builds>
  <https://nix.dev/guides/recipes/add-binary-cache.html>
- Service installation needs platform-native packaging: launchd/LaunchAgent on
  macOS, systemd user services on Linux, and a per-user Windows strategy.
  <https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html>
  <https://www.freedesktop.org/software/systemd/man/systemd.service.html>
  <https://learn.microsoft.com/en-us/windows/application-management/per-user-services-in-windows>
  <https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes>

## 2. Current Baseline

Already implemented or substantially implemented:

- Rust workspace with `devrelay-core`, `devrelay-cli`, and `devrelay-agent`.
- Typed manifest parsing, validation, and command trust hashes.
- Git status collection through the installed Git CLI.
- Safe untracked classification and secret hard-blocks.
- Synthetic snapshot metadata and local snapshot create/apply/verify.
- Local project registry, workspace mapping, SQLite metadata, and migrations.
- Per-project bare snapshot repository.
- Recovery list/show/open and dirty target policies.
- Local handoff simulator.
- Agent JSON-RPC, macOS/Linux Unix socket IPC, event stream, structured logs,
  and diagnostics.
- Anchor metadata, sessions, lease state machine, canonical publish, handoff,
  inactive edit fork, and crash recovery.
- Fabric/device identity, pairing, mDNS discovery, mTLS transport primitives,
  revocation, and audit logging.
- Git object data plane using per-project bare repos.
- CAS sidecars, route selection, partial upload safety, and anchor snapshot
  repos.
- Background debounce, checkpointing, retention, quota, resource policy, and
  crash journal.
- Environment trust/profile selection plus Nix and Dev Container adapters.
- Cross-platform path, line-ending, symlink, reparse, and WSL doctors.
- Advanced Git handling for conflicts, submodules, LFS, sparse checkout, and
  gated sequencer/rebase states.

Current gaps that matter before product claims:

- Windows named pipe IPC and pipe ACL.
- Linux/Windows watcher backend completeness.
- M4.5 Control API endpoints and schema, or a renamed remote RPC boundary.
- Idle CPU/RSS and checkpoint burst evidence.
- Product-level safety suites named from the non-negotiable invariants.
- Production desktop UI.
- Extension-host validation for editor context restore.
- Native bootstrap, secret providers, hydration state machine, and environment
  doctor.
- Compute fabric.
- Signed release/update story and beta onboarding.

## 3. Execution Model

### Workstream A: Core Correctness

Owns Git state, snapshot semantics, apply verification, operation capsules,
sidecar CAS, path safety, and round-trip tests.

### Workstream B: Local Agent And Anchor

Owns the long-running service, local IPC, SQLite metadata, lease state machine,
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

Status: complete.

Goal: prove the central technical claim: supported Git states can be captured,
applied to another clone, and verified without changing user history.

Completed capabilities:

- Manifest parsing and validation.
- Git porcelain v2 status parser.
- Snapshot metadata schema and state hash.
- Secret/untracked policy.
- Synthetic index/work commits.
- Apply and verification.
- Round-trip fixtures.
- Stable CLI JSON and error output.

Exit gate: closed.

### M1. Local CLI MVP

Status: complete.

Goal: a single user can register projects and explicitly checkpoint, list,
recover, and apply sessions on one machine or local paths.

Completed capabilities:

- `DEVRELAY_HOME` layout.
- Local config and project registry.
- SQLite metadata with migrations.
- Workspace mapping.
- Per-project bare snapshot store.
- Recovery CLI.
- Dirty target backup policies.
- Stable error schema.
- Local `continue` simulator.

Exit gate: closed.

### M2. Agent And Local IPC

Status: nearly complete.

Goal: convert CLI-only logic into a user-session agent that owns state and
exposes stable local APIs to CLI, UI, and future editor plugins.

Completed capabilities:

- `devrelay-agent`.
- macOS/Linux Unix domain socket IPC.
- JSON-RPC method set and compatibility policy.
- CLI routing through agent.
- Event stream with replay cursor and gap detection.
- Structured logs and diagnostic bundle.
- macOS LaunchAgent and Linux systemd user templates.

Open work:

- Windows named pipe transport.
- Per-user Windows pipe ACL.
- Product decision on whether M7 first dogfood is macOS/Linux-only or blocks on
  Windows IPC first.

Exit gate: open for 3-OS support.

### M3. Anchor Metadata And Single-Writer Lease

Status: complete.

Goal: introduce the control-plane concept that makes DevRelay distinct from Git
sync: one canonical writer and safe forks for stale/inactive work.

Completed capabilities:

- Anchor mode and metadata schema.
- Device identity placeholders.
- Sessions.
- Lease state machine.
- Canonical publish with stale update rejection.
- Handoff protocol.
- Inactive edit fork.
- Crash recovery.

Exit gate: closed.

### M4. LAN Pairing And Secure Control Plane

Status: mostly complete, but exit gate is open.

Goal: make multiple devices usable without manual trust shortcuts.

Completed capabilities:

- Fabric identity.
- Pairing protocol with short authentication string.
- mDNS discovery with privacy-constrained TXT records.
- mTLS transport primitives and replay controls.
- Revocation.
- Audit log.

Open work:

- M4.5 Control API server/endpoints/auth middleware/schema/tests.
- Decide whether the remote control boundary is HTTP `/v1` or versioned RPC
  over mTLS. If it is RPC, rename M4.5 and update the API docs.

Exit gate: open until unauthenticated Control API requests are rejected by an
implemented API boundary with integration tests.

### M5. Data Plane: Git Object Transfer And Sidecar CAS

Status: complete.

Goal: stop relying on source workspace filesystem paths and support
direct/anchor object transfer for real devices.

Completed capabilities:

- Local bare repo data-plane strategy.
- Authorized serve plan limited to DevRelay refs.
- Anchor snapshot repos.
- Direct/anchor route selection.
- CAS chunk store and manifests.
- Large sidecars.
- Atomic sidecar materialization.
- Partial upload safety.

Exit gate: closed.

### M6. Background Protection

Status: nearly complete.

Goal: make DevRelay quietly protective without becoming noisy or resource-heavy.

Completed capabilities:

- Watcher trait, macOS watcher, polling fallback, event coalescing.
- Adaptive debounce.
- Background checkpointing.
- Git performance doctor.
- Resource policy.
- Retention and quota.
- Crash journal.

Open work:

- Linux watcher backend.
- Windows watcher backend.
- Idle CPU/RSS benchmark results.
- Checkpoint burst CPU/RSS benchmark results.
- Watch event versus actual scan evidence.

Exit gate: open until [resource-benchmark.md](resource-benchmark.md) has
representative measurements.

### M7. Desktop UX: Tray And Dashboard

Status: started. The Tauri tray/dashboard first slice, Continue screen,
handoff dialog, Activity/Settings surfaces, and agent-backed handoff
prepare/continue paths exist. Real macOS/Linux device dogfood remains open.

Goal: ship the day-to-day product surface around "Continue here" without
exposing Git plumbing by default.

First slice:

```text
Mac에서 작업하던 프로젝트를 Linux에서 두 번의 클릭으로 이어서 연다.
```

Required first surfaces:

- Tray/menu bar.
- Continue screen.
- Handoff dialog.
- Dirty target preservation screen.
- Completion screen.
- Activity details entry point.

Out of first slice:

- Runs full screen.
- Scheduler UI.
- CAS details.
- Lease epoch.
- Git OIDs.
- Pairing certificate details.
- Nix store internals.
- Advanced retention controls.
- Graphs and statistics.

Exit gate:

- Same-LAN handoff starts from tray in two clicks.
- Clean target handoff works on real macOS/Linux devices.
- Dirty target flow preserves work without Git terminology.
- UI state comes from the agent event/RPC boundary.
- Keyboard-only core flow works.
- Screen reader labels cover primary actions.

See [ui-vertical-slice.md](ui-vertical-slice.md).

### M8. Editor Context: VS Code First

Status: started.

Goal: restore human context after handoff without pretending process state can
move across operating systems.

Keep this after verified code handoff. Editor context is valuable, but it must
not block the first proof that code and writer authority move safely.

Completed capabilities:

- VS Code extension package skeleton.
- TypeScript build, local lint, and Node test scripts.
- Local agent IPC client using length-prefixed JSON-RPC.
- Status bar connection state and refresh/explain commands.
- Package metadata and client protocol tests.
- Workspace folder, opened tab order, active editor, cursor, selection, split,
  breakpoint, and terminal title/cwd capture from VS Code public APIs.
- Size-limited editor context capsule upload through local
  `editor.context.update` RPC with durable audit evidence.
- User-controlled editor context capture through
  `devrelay.captureEditorContext`; code handoff command flow can still start
  when context capture is skipped or fails.
- Unsaved dirty buffer capture and restore through local VS Code SecretStorage,
  default-off settings, untitled-buffer opt-in, and dirty untitled restore.
- Status bar active/inactive/handoff/protection-delayed states from agent
  leases and handoffs, plus explain and dashboard commands.
- Handoff edit guard: VS Code edit/save/active-editor events report to the
  agent, meaningful edits increment source generation, and source-side pending
  handoffs abort on generation change.
- Context restore command: latest captured context retrieval, saved
  folder/file/tab-order reopening, active selections, split columns,
  breakpoints, local dirty-buffer restore, restore ACK audit, partial detail
  reporting, and Node/Rust coverage.
- Command palette actions for continue here, continue elsewhere,
  checkpoint, run task, recovery timeline, explain state, refresh,
  dashboard, capture, restore, and unsaved-buffer commands. Continue
  elsewhere captures context and starts `handoff.begin` only when a registered
  project, active local writer lease, and target device are available. Run task
  currently opens recent run history because the local agent has no task-start
  RPC yet.

Open work:

- Extension-host validation inside VS Code.

### M9. Environment Hydration

Status: partial.

Goal: make "code ready" and "environment ready" explicit, reproducible, and
safe.

Completed capabilities:

- Trust hash model for manifest commands and declared bootstrap fingerprint
  file contents.
- Profile selection.
- Nix adapter.
- Dev Container adapter.
- Native bootstrap adapter with POSIX/PowerShell command classification, trust
  approval gate, timeout, redacted output capture, healthcheck reporting, and
  idempotency guidance.
- Secret provider mapping model with OS keychain design, 1Password CLI,
  Bitwarden CLI, SOPS/age, and user-script provider plans; local secret file
  and environment materialization; required-secret errors; redacted reports; and
  hard exclusion of manifest secret file targets from snapshots.
- Hydration state machine from cold through app-ready, failed/retry handling,
  JSON state persistence, and `environment.progress` event payloads.
- Environment doctor report and `devrelay doctor environment` CLI for missing
  Nix, missing container engines, missing PowerShell, changed command hashes,
  missing required secrets, incompatible platform targets, and opt-in
  healthcheck failures with safe actions.

Open work:

- CLI/UI visibility for hydration state.

### M10. Compute Fabric

Status: partial.

Goal: use idle personal machines for builds, tests, benchmarks, and agent work
without taking writer ownership.

Completed capabilities:

- Task definitions parse from `devrelay.toml` and validate profile references,
  commands, platform constraints, resource hints, and sandbox settings.
- Task-specific command definition hashes include the task command, task
  constraints, resource hints, and selected environment profile definition.
- Immutable task execution snapshots can be created as pinned snapshots tied to
  the task definition hash.
- Scheduler constraint filtering can collect static/dynamic device resources and
  reject incompatible platforms, missing features, insufficient resources, or
  locally disallowed devices.
- Explainable scheduler scoring normalizes cache warmth, CPU, memory, power,
  data locality, network quality, historical speed, user affinity, transfer
  cost, foreground load, and thermal placeholder signals with task-class
  weights.
- Isolated runner workspaces apply immutable task execution snapshots from the
  snapshot store, gate the task environment profile, materialize required
  sidecars/secrets only when permitted, stay non-canonical, and clean up through
  a retention policy.
- Host task execution can run inside the prepared workspace with working
  directory and environment handling, timeout enforcement, stdout/stderr capture,
  live log sink events, process-tree cancellation on timeout, and explicit
  sandbox/container/VM placeholders.
- Task logs can be streamed through a bounded live buffer, redacted, spooled to
  per-run JSONL files, retrieved later, and marked with a truncation record when
  spool limits are reached.
- Declared task outputs can be captured as artifacts, path-checked, hashed into
  CAS manifests, indexed per task run, pulled on demand, and released through
  artifact retention roots.
- Task result cache keys include snapshot trees, sidecar inputs, environment
  fingerprint, command hash, platform, and outputs; cache metadata stores
  artifact indexes, returns hits, restores artifacts from CAS, and disables
  secret-sensitive tasks by default without accepting secret values.
- Task run metadata can be recorded and listed from the per-project metadata
  database.

Open work:

- Remote runner, Nix delegation, and code-changing task integration.

### M11. Cross-Platform Hardening

Status: mostly complete.

Goal: make macOS, Linux, Windows native, and WSL behavior predictable.

Completed capabilities:

- Platform identity.
- Path portability doctor.
- Line-ending policy checks.
- Executable bit verification.
- Symlink policy.
- Windows reparse defenses.
- WSL/native separation.

Open work:

- Convert current platform checks into broad real-device dogfood evidence.
- Avoid broad Windows UI claims until M2 Windows IPC is complete.

### M12. Advanced Git States

Status: mostly complete.

Goal: expand beyond normal index states without compromising correctness.

Completed capabilities:

- Merge/cherry-pick/revert conflict capture.
- Index stage preservation.
- Submodules.
- LFS.
- Sparse checkout and partial clone handling.
- Interactive rebase/sequencer detection and safe block fallback.

Open work:

- Interactive rebase reconstruction remains disabled until exhaustive tests are
  complete.

### M13. Security, Privacy, And Operations Gate

Status: partial.

Goal: get the system ready for real daily use on private machines.

Completed or started:

- Fuzzing targets and corpus seeds.
- Secret scanning primitives.
- Diagnostic bundle.
- Fault injection.
- Migration tests from old fixtures.

Open work:

- Threat model.
- False-positive override design.
- Encrypted one-time sidecar design.
- Release channel, signed release, and provenance strategy.
- Backup anchor.
- Opaque anchor research decision.
- Independent security review.

### M14. Beta Product Loop

Status: not started.

Goal: make the product reliable for real daily development before expanding
scope.

Requires:

- installers/dev channels
- onboarding
- guided doctor
- local metrics
- user docs
- real-device dogfood

### M15. North Star Release Candidate

Status: not started.

Goal: meet the final product definition from the spec.

Requires:

- all product capabilities
- verified continuation gate
- correctness gate
- resource gate
- UX gate
- security gate
- signed/reproducible-enough artifacts

## 5. Immediate Sequence

The next sequence is product-first but still safety-gated:

1. Finish document drift cleanup and keep the checklist honest.
2. Decide macOS/Linux-first dogfood versus Windows IPC first.
3. Measure idle resource behavior.
4. Add named safety integration suites.
5. Scaffold Tauri shell and subscribe to agent state/events.
6. Keep tray and Continue screen aligned with agent state during runtime checks.
7. Dogfood clean-target handoff on real macOS/Linux devices.
8. Dogfood dirty-target and inactive-edit preservation.
9. Add Windows/WSL once Windows IPC/startup are credible.
10. Expand Projects, Devices, Recovery timeline, Activity, and Settings.

## 6. Non-Negotiables

- No silent overwrite.
- No automatic merge in background.
- No plaintext secret snapshot by default.
- No remote command execution without trust hash approval.
- No UI state that bypasses the agent's source of truth.
- No background watcher treated as truth.
- No cross-device handoff success until verification passes.
- No compute task writes directly into the active session.

The policy and evidence mapping live in
[data-loss-safety.md](data-loss-safety.md).
