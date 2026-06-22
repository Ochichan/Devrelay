# DevRelay North Star Checklist

Last updated: 2026-06-23

This is the execution checklist for `docs/north-star-roadmap.md`. The roadmap
explains why and when. This file tracks what must be built, tested, documented,
and gated from the current foundation through the North Star release candidate.

Conventions:

- `[ ]` means not done.
- `[x]` means complete and verified.
- Keep items small enough to become a GitHub issue or a focused PR.
- Do not mark a milestone complete until its exit-gate checklist is complete.
- Do not skip safety, recovery, or verification items to accelerate UI work.

## Global Project Setup

### Repository Hygiene

- [x] Initialize this directory as a Git repository when ready to track project changes.
- [x] Add a branch naming convention for implementation milestones.
- [x] Add a PR template with correctness, safety, and test sections.
- [x] Add an issue template for roadmap checklist items.
- [x] Add an issue template for bugs involving data safety.
- [x] Add an issue template for security/privacy findings.
- [x] Add a changelog file.
- [x] Add a release notes file or process.
- [x] Add a `CONTRIBUTING.md` with local setup instructions.
- [x] Add a `SECURITY.md` with responsible reporting instructions.
- [x] Add a `docs/decisions/` directory for architecture decision records.
- [x] Add an ADR template.
- [x] Record the decision to use Rust core and Git CLI orchestration.
- [x] Record the decision to make the agent the only state authority for UI.
- [x] Record the decision to keep GUI work after core verification gates.

### Build And CI

- [x] Add a local `justfile`, `makefile`, or documented command set.
- [x] Add a command for `cargo fmt --all -- --check`.
- [x] Add a command for `cargo clippy --workspace --all-targets -- -D warnings`.
- [x] Add a command for `cargo test --workspace`.
- [x] Add a command for all local preflight checks.
- [x] Add CI for macOS.
- [x] Add CI for Linux.
- [x] Add CI for Windows.
- [x] Cache Cargo dependencies in CI.
- [x] Run formatting in CI.
- [x] Run clippy in CI.
- [x] Run unit tests in CI.
- [x] Run integration tests in CI.
- [ ] Publish CI artifacts for failed integration test logs.
- [x] Add minimum Rust version enforcement.
- [ ] Add dependency audit tooling.
- [ ] Add license/dependency inventory tooling.

### Documentation Baseline

- [x] Keep `README.md` updated with current runnable commands.
- [x] Keep `docs/foundation.md` aligned with current implementation.
- [ ] Keep `docs/north-star-roadmap.md` aligned with strategic scope.
- [ ] Keep this checklist aligned with roadmap milestones.
- [ ] Add a glossary for Fabric, Device, Project, Workspace, Session, Snapshot, Lease, and Capsule.
- [ ] Add a "supported states" document.
- [ ] Add an "unsupported states" document.
- [ ] Add a data-loss safety policy document.
- [ ] Add a user-facing recovery policy document.
- [ ] Add a developer-facing testing strategy document.

## M0. Foundation: Local Git Round Trip

### M0.1 Existing Foundation Cleanup

- [x] Review current `devrelay-core` public API shape.
- [x] Decide which current structs are public stable API.
- [x] Move unstable structs behind internal modules where appropriate.
- [x] Add crate-level documentation for `devrelay-core`.
- [x] Add crate-level documentation for `devrelay-cli`.
- [x] Add module documentation for manifest parsing.
- [x] Add module documentation for Git orchestration.
- [x] Add module documentation for untracked/secret policy.
- [x] Add module documentation for snapshots.
- [x] Add examples to Rust docs where useful.

### M0.2 Manifest Validation

- [x] Add golden test for bundled `devrelay.toml`.
- [x] Add negative test for missing `schema`.
- [x] Add negative test for unsupported `schema`.
- [x] Add negative test for short `project_id`.
- [x] Add negative test for empty project `name`.
- [x] Add negative test for overlong project `name`.
- [x] Add negative test for invalid `workspace.untracked`.
- [x] Add negative test for invalid `workspace.portable_paths`.
- [x] Add negative test for invalid `large_file_threshold_mib`.
- [x] Add negative test for duplicate exclude patterns.
- [x] Add negative test for duplicate include patterns.
- [x] Add negative test for empty environment profile command.
- [x] Add negative test for empty environment profile target.
- [x] Add negative test for empty task command.
- [x] Add negative test for empty task profile.
- [x] Add negative test for empty secret target.
- [x] Add serde round-trip test for manifest structs.
- [x] Add canonical execution-trust hash calculation for commands.
- [x] Add tests proving non-executable manifest edits do not change trust hash.
- [x] Add tests proving command edits do change trust hash.

### M0.3 Git Status Parser

- [x] Parse branch OID header.
- [x] Parse branch head header.
- [x] Parse detached HEAD correctly.
- [x] Parse upstream header.
- [x] Ignore unknown headers.
- [x] Parse ordinary changed entries.
- [x] Parse staged add entries.
- [x] Parse staged modify entries.
- [x] Parse staged delete entries.
- [x] Parse unstaged modify entries.
- [x] Parse unstaged delete entries.
- [x] Parse rename entries.
- [x] Parse copy entries if reported by Git.
- [x] Parse untracked entries.
- [x] Parse ignored entries.
- [x] Parse unmerged entries into a blocked state for M0.
- [x] Preserve NUL-delimited paths exactly as UTF-8 lossless policy allows.
- [x] Add test for spaces in paths.
- [x] Add test for tab characters in paths if Git emits them safely.
- [x] Add test for Unicode paths.
- [x] Add test for unknown porcelain v2 header.
- [x] Add test for initial repository with no commits and define behavior.
- [x] Add stable status summary struct for CLI JSON output.

### M0.4 Snapshot Metadata Schema

- [x] Create explicit snapshot schema module.
- [x] Add `schema_version`.
- [x] Add stable snapshot ID format.
- [x] Add project ID.
- [x] Add project name.
- [x] Add session ID placeholder or explicit absence.
- [x] Add parent snapshot ID placeholder or explicit absence.
- [x] Add source device ID placeholder or explicit absence.
- [x] Add branch name.
- [x] Add HEAD OID.
- [x] Add index tree OID.
- [x] Add index commit OID.
- [x] Add work tree OID.
- [x] Add work commit OID.
- [x] Add status counts.
- [x] Add included untracked manifest.
- [x] Add excluded path manifest.
- [x] Add state hash.
- [x] Add created timestamp.
- [x] Add snapshot metadata JSON golden fixture.
- [x] Add schema backward compatibility test for current version.
- [x] Add JSON serialization test with stable field names.
- [x] Add JSON deserialization test from golden fixture.
- [x] Add validation for empty required OIDs.
- [x] Add validation for malformed snapshot IDs.

### M0.5 State Hashing

- [x] Define canonical state hash input fields.
- [x] Exclude non-semantic fields from state hash.
- [x] Include project ID in state hash.
- [x] Include HEAD OID in state hash.
- [x] Include index tree OID in state hash.
- [x] Include work tree OID in state hash.
- [x] Include included untracked manifest in state hash.
- [x] Include excluded path reasons in state hash.
- [x] Add deterministic ordering for included paths.
- [x] Add deterministic ordering for excluded paths.
- [x] Add test proving path order does not change hash after canonical sort.
- [x] Add test proving content/tree changes do change hash.

### M0.6 Secret And Untracked Policy

- [x] Add default hard-block for `.env`.
- [x] Add default hard-block for `.env.*`.
- [x] Add default hard-block for `.ssh/**`.
- [x] Add default hard-block for private key extensions.
- [x] Add private key header detection tests.
- [x] Add high-entropy detection placeholder behind disabled feature.
- [x] Add generated directory exclude for `node_modules/**`.
- [x] Add generated directory exclude for `.venv/**`.
- [x] Add generated directory exclude for `target/**`.
- [x] Add generated directory exclude for `dist/**`.
- [x] Add generated directory exclude for `.next/**`.
- [x] Add socket/PID/lock file exclude tests.
- [x] Add large file threshold exclude test.
- [x] Add `untracked = none` test.
- [x] Add `untracked = safe` test.
- [x] Add `untracked = all-nonignored` test.
- [x] Add `untracked = explicit` include-pattern test.
- [x] Add classification reason codes.
- [x] Add CLI display for included/excluded untracked paths.
- [x] Add JSON output for untracked classification.

### M0.7 Synthetic Snapshot Creation

- [x] Confirm checkpoint does not alter working tree.
- [x] Confirm checkpoint does not alter source index.
- [x] Write index tree from current index.
- [x] Create synthetic index commit parented by HEAD.
- [x] Build temporary index for work tree.
- [x] Stage unstaged tracked changes into temporary index.
- [x] Stage accepted untracked files into temporary index.
- [x] Preserve staged/unstaged split.
- [x] Create synthetic work commit parented by index commit.
- [x] Store refs under `refs/devrelay/snapshots/<id>/index`.
- [x] Store refs under `refs/devrelay/snapshots/<id>/work`.
- [x] Add guard for unmerged index in M0.
- [x] Add guard for unsupported rebase/sequencer state in M0.
- [x] Add guard for missing HEAD or unborn branch.
- [x] Add cleanup for temporary index files.
- [x] Add test proving temporary files are removed.

### M0.8 Apply And Verification

- [x] Refuse dirty target before apply.
- [x] Fetch required snapshot refs from source path.
- [x] Check out target branch if snapshot has branch.
- [x] Check out detached HEAD if snapshot is detached.
- [x] Reset target to source HEAD.
- [x] Apply work tree from work commit.
- [x] Reset index to index commit.
- [x] Verify target HEAD OID.
- [x] Verify target index tree OID.
- [x] Verify target work tree OID.
- [x] Verify state hash.
- [x] Verify included untracked paths exist where expected.
- [x] Verify excluded secret paths were not materialized.
- [x] Return structured verification details.
- [x] Add `apply --dry-run`.
- [x] Add `apply --json`.
- [x] Add stable error for dirty target.
- [x] Add stable error for missing source object.
- [x] Add stable error for verification mismatch.

### M0.9 Round-Trip Fixtures

- [x] Add fixture helper for temporary Git repos.
- [x] Add fixture helper for source/target clone setup.
- [x] Add fixture for staged add.
- [x] Add fixture for staged modify.
- [x] Add fixture for staged delete.
- [x] Add fixture for unstaged modify.
- [x] Add fixture for unstaged delete.
- [x] Add fixture for staged delete plus same-path recreation if supported.
- [x] Add fixture for untracked file include.
- [x] Add fixture for untracked secret exclude.
- [x] Add fixture for binary file modify.
- [x] Add fixture for empty file.
- [x] Add fixture for executable bit on POSIX.
- [x] Add fixture for Unicode path.
- [x] Add fixture for path with spaces.
- [x] Add fixture for rename.
- [x] Add fixture for ignored generated directory exclusion.
- [x] Add fixture for dirty target refusal.
- [x] Add fixture for source status unchanged after checkpoint.
- [x] Add fixture for status equivalence after apply.

### M0.10 CLI Polish

- [x] Add `devrelay --version`.
- [x] Add subcommand help examples.
- [x] Add `manifest check --json`.
- [x] Add `status --json` stable schema.
- [x] Add `checkpoint --json` stable schema.
- [x] Add `apply --json` stable schema.
- [x] Add pretty error output for terminal users.
- [x] Add JSON error output for scripts.
- [x] Add nonzero exit code conventions.
- [x] Add CLI snapshot file path default documentation.
- [x] Add CLI integration tests with `assert_cmd` or equivalent.

### M0 Exit Gate

- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [x] Run `cargo test --workspace`.
- [x] Run local round-trip fixture suite.
- [x] Verify source repository remains unchanged after checkpoint.
- [x] Verify target status equivalence after apply.
- [x] Verify dirty target stable error code.
- [x] Update `docs/foundation.md`.
- [x] Update `README.md` quick start if commands changed.

## M1. Local CLI MVP

### M1.1 Local Data Directory

- [x] Define `$DEVRELAY_HOME` resolution.
- [x] Add default path for macOS.
- [x] Add default path for Linux.
- [x] Add default path for Windows.
- [x] Add `DEVRELAY_HOME` env override.
- [x] Add config file path helper.
- [x] Add project data path helper.
- [x] Add snapshot bare repo path helper.
- [x] Add CAS path helper.
- [x] Add log path helper.
- [x] Add diagnostics path helper.
- [x] Create directories atomically.
- [x] Add permissions check for data directory.
- [x] Add test for data directory creation.
- [x] Add test for custom `DEVRELAY_HOME`.

### M1.2 Local Config

- [x] Define local config TOML schema.
- [x] Add config version field.
- [x] Add default fabric name.
- [x] Add default device name.
- [x] Add default editor preference field.
- [x] Add resource profile field.
- [x] Add anchor mode field.
- [x] Add project registry index.
- [x] Add config load command.
- [x] Add config save command.
- [x] Add config migration placeholder.
- [x] Add config validation.
- [x] Add config redaction for diagnostics.
- [x] Add config tests.

### M1.3 SQLite Metadata

- [x] Add SQLite dependency.
- [x] Create migration runner.
- [x] Create metadata DB file.
- [x] Enable WAL mode.
- [x] Create `schema_migrations` table.
- [x] Create `projects` table.
- [x] Create `workspaces` table.
- [x] Create `sessions` table.
- [x] Create `snapshots` table.
- [x] Create `leases` table placeholder.
- [x] Create `handoffs` table placeholder.
- [x] Create indexes for project lookup.
- [x] Create indexes for session snapshot timeline.
- [x] Add transaction helper.
- [x] Add migration test for empty DB.
- [x] Add migration idempotency test.
- [x] Add migration rollback policy documentation.

### M1.4 Project Registry

- [x] Add `devrelay project add <path>`.
- [x] Resolve Git repository root.
- [x] Detect non-Git path and return stable error.
- [x] Load `devrelay.toml` from project root if present.
- [x] Accept `--manifest <path>`.
- [x] Generate project ID when manifest absent.
- [x] Use manifest project ID when present.
- [x] Capture project display name.
- [x] Capture canonical remote URL fingerprint.
- [x] Capture root commit fingerprint where possible.
- [x] Add workspace record for current path.
- [x] Prevent duplicate workspace registration.
- [x] Add `devrelay projects list`.
- [x] Add `devrelay project show <id-or-name>`.
- [x] Add `devrelay project remove <id-or-name>`.
- [x] Add JSON output for project commands.
- [x] Add project registry integration tests.

### M1.5 Workspace Mapping

- [x] Define workspace ID format.
- [x] Store device ID placeholder.
- [x] Store local path.
- [x] Store platform profile.
- [x] Store workspace state.
- [x] Store last seen HEAD.
- [x] Store last checkpoint ID.
- [x] Add workspace lookup by path.
- [x] Add workspace lookup by project/device/path.
- [x] Add duplicate path protection.
- [x] Add stale path detection.
- [x] Add workspace remove command.
- [x] Add workspace tests.

### M1.6 Snapshot Store

- [x] Create per-project bare snapshot repo.
- [x] Initialize bare repo lazily.
- [x] Store synthetic snapshot refs in snapshot repo where feasible.
- [x] Add object import/export helper.
- [x] Keep main repo refs minimal.
- [x] Persist snapshot metadata in SQLite.
- [x] Persist parent snapshot ID.
- [x] Persist sequence number.
- [x] Persist pinned flag.
- [x] Persist label.
- [x] Add `checkpoint --label`.
- [x] Add `checkpoint --pin`.
- [x] Add snapshot list query.
- [x] Add snapshot show query.
- [x] Add snapshot JSON export.
- [x] Add snapshot store tests.

### M1.7 Recovery CLI

- [x] Add `devrelay recover list`.
- [x] Add `devrelay recover list --project`.
- [x] Add `devrelay recover show <snapshot-id>`.
- [x] Add `devrelay recover open <snapshot-id> --path <new-workspace>`.
- [x] Refuse recovery into existing dirty path.
- [x] Create new workspace directory if missing.
- [x] Clone/fetch base repository into recovery workspace.
- [x] Apply snapshot to recovery workspace.
- [x] Register recovery workspace if requested.
- [x] Add `--name` for recovery session.
- [x] Add JSON output.
- [x] Add tests for non-destructive recovery.
- [x] Add tests for missing snapshot.
- [x] Add tests for dirty recovery target.

### M1.8 Dirty Target Backup

- [x] Detect target dirty status before apply.
- [x] Add `--dirty-policy block`.
- [x] Add `--dirty-policy snapshot-and-fork`.
- [x] Add `--dirty-policy new-workspace`.
- [x] Implement local target backup snapshot.
- [x] Persist backup snapshot as fork session.
- [x] Pin backup snapshot by default.
- [x] Add CLI output explaining backup.
- [x] Add JSON output for safe actions.
- [x] Add test proving local dirty file is preserved.
- [x] Add test proving source snapshot still applies after backup.

### M1.9 Stable Error Schema

- [x] Define error code namespace.
- [x] Add `DR-MANIFEST-*` codes.
- [x] Add `DR-GIT-*` codes.
- [x] Add `DR-SNAPSHOT-*` codes.
- [x] Add `DR-APPLY-*` codes.
- [x] Add `DR-RECOVER-*` codes.
- [x] Add `DR-STORAGE-*` codes.
- [x] Add title field.
- [x] Add detail field.
- [x] Add safe actions field.
- [x] Add diagnostic ID field.
- [x] Add terminal rendering.
- [x] Add JSON rendering.
- [x] Add tests for representative errors.

### M1.10 Local Handoff Simulator

- [x] Add `devrelay continue --source <path> --target <path>`.
- [x] Resolve source workspace.
- [x] Resolve target workspace.
- [x] Create final source checkpoint.
- [x] Backup dirty target if policy allows.
- [x] Apply source snapshot to target.
- [x] Verify target.
- [x] Mark source inactive in local metadata placeholder.
- [x] Mark target active in local metadata placeholder.
- [x] Add `--dry-run`.
- [x] Add JSON output.
- [x] Add integration test for clean target.
- [x] Add integration test for dirty target backup.

### M1 Exit Gate

- [x] User can add a project without manual config edits.
- [x] User can checkpoint a registered project.
- [x] User can list snapshots.
- [x] User can recover a snapshot into a new workspace.
- [x] User can run local handoff simulator.
- [x] Local apply never overwrites dirty target data.
- [x] SQLite migrations have tests.
- [x] CLI JSON schemas are documented.
- [x] All M1 tests pass in CI.

## M2. Agent And Local IPC

### M2.1 Agent Binary

- [x] Add `crates/devrelay-agent`.
- [x] Add agent CLI flags.
- [x] Add `--foreground`.
- [x] Add `--config`.
- [x] Add `--socket-path`.
- [x] Add `--log-level`.
- [x] Add graceful shutdown handler.
- [x] Add startup config load.
- [x] Add startup database migration.
- [x] Add startup project registry load.
- [x] Add health endpoint.
- [x] Add agent unit tests.

### M2.2 Local IPC Transport

- [x] Define IPC transport trait.
- [x] Implement Unix domain socket transport.
- [ ] Implement Windows named pipe transport.
- [x] Add local socket path resolution.
- [x] Add stale socket cleanup.
- [x] Add peer credential check on Unix where available.
- [ ] Add pipe access control on Windows.
- [x] Add connection timeout.
- [x] Add request timeout.
- [x] Add max message size.
- [x] Add malformed message handling.
- [x] Add IPC integration tests on current platform.

### M2.3 JSON-RPC API

- [x] Define RPC envelope.
- [x] Define request ID behavior.
- [x] Define error envelope.
- [x] Define version negotiation method.
- [x] Add `agent.health`.
- [x] Add `status.get`.
- [x] Add `projects.add`.
- [x] Add `projects.list`.
- [x] Add `projects.show`.
- [x] Add `checkpoint.create`.
- [x] Add `snapshots.list`.
- [ ] Add `recover.open`.
- [ ] Add `apply.snapshot`.
- [x] Add `diagnostics.export`.
- [x] Add RPC schema tests.
- [x] Add backwards compatibility policy.

### M2.4 CLI Through Agent

- [ ] Add client library for local agent RPC.
- [ ] Add CLI global `--direct` escape hatch.
- [ ] Add CLI global `--agent-socket`.
- [ ] Route `status` through agent.
- [ ] Route project commands through agent.
- [ ] Route checkpoint through agent.
- [ ] Route recover through agent.
- [ ] Route apply through agent.
- [ ] Preserve CLI JSON output compatibility.
- [ ] Add fallback message when agent is unavailable.
- [ ] Add integration test with spawned agent.

### M2.5 Event Stream

- [ ] Define event envelope.
- [ ] Add monotonic event sequence.
- [ ] Add event timestamp.
- [ ] Add event replay cursor.
- [ ] Add `workspace.state.changed`.
- [ ] Add `snapshot.local.created`.
- [ ] Add `snapshot.apply.started`.
- [ ] Add `snapshot.apply.verified`.
- [ ] Add `security.blocked`.
- [ ] Add `quota.warning`.
- [ ] Add subscription RPC.
- [ ] Add reconnect behavior.
- [ ] Add gap detection.
- [ ] Add event stream tests.

### M2.6 Structured Logs

- [ ] Add JSON line log format.
- [ ] Add human dev log format.
- [ ] Add log rotation.
- [ ] Add log retention.
- [ ] Add request ID in logs.
- [ ] Add operation ID in logs.
- [ ] Add redaction helper.
- [ ] Redact secret values.
- [ ] Redact credentialed remote URLs.
- [ ] Redact local paths in diagnostic mode when requested.
- [ ] Add log tests for redaction.

### M2.7 Service Templates

- [ ] Add macOS LaunchAgent template.
- [ ] Add Linux systemd user service template.
- [ ] Add Windows background process/service design note.
- [ ] Add `devrelay agent install --dry-run`.
- [ ] Add `devrelay agent install` for macOS dev mode.
- [ ] Add `devrelay agent install` for Linux dev mode.
- [ ] Add `devrelay agent uninstall`.
- [ ] Add `devrelay agent status`.
- [ ] Document manual Windows startup setup until packaged.
- [ ] Add service template tests where practical.

### M2.8 Diagnostics

- [ ] Add diagnostic bundle command.
- [ ] Include version/capability.
- [ ] Include redacted config.
- [ ] Include recent structured logs.
- [ ] Include state machine records placeholder.
- [ ] Include Git command exit codes.
- [ ] Include timing data.
- [ ] Exclude source code by default.
- [ ] Exclude snapshot objects by default.
- [ ] Add `--include-sensitive-paths` explicit option.
- [ ] Add diagnostic redaction tests.

### M2 Exit Gate

- [ ] CLI can operate entirely through the agent.
- [ ] Agent restart preserves project and snapshot state.
- [ ] IPC access is scoped to the local user.
- [ ] Event stream reconnect works.
- [ ] Diagnostic bundle excludes source code by default.
- [ ] Agent tests pass in CI.

## M3. Anchor Metadata And Single-Writer Lease

### M3.1 Anchor Mode

- [ ] Add anchor mode config field.
- [ ] Add `devrelay anchor init`.
- [ ] Add `devrelay anchor status`.
- [ ] Add anchor data directory layout.
- [ ] Add anchor metadata DB path.
- [ ] Add anchor snapshot repo root.
- [ ] Add anchor CAS root.
- [ ] Add anchor startup path.
- [ ] Add agent role detection.
- [ ] Add role-specific health output.

### M3.2 Metadata Schema

- [ ] Add `devices` table.
- [ ] Add `projects` table compatibility with local registry.
- [ ] Add `workspaces` table with device ID.
- [ ] Add `sessions` table.
- [ ] Add `snapshots` table with sequence number.
- [ ] Add `leases` table.
- [ ] Add `handoffs` table.
- [ ] Add `task_runs` placeholder table.
- [ ] Add foreign keys where appropriate.
- [ ] Add indexes for lease lookup.
- [ ] Add indexes for latest snapshot lookup.
- [ ] Add indexes for handoff lookup.
- [ ] Add schema migration tests.
- [ ] Add WAL mode test.

### M3.3 Device Identity Placeholder

- [ ] Generate local device ID.
- [ ] Store device display name.
- [ ] Store platform key.
- [ ] Store architecture.
- [ ] Store capabilities JSON.
- [ ] Store paired timestamp placeholder.
- [ ] Store last seen timestamp.
- [ ] Add `devrelay devices list`.
- [ ] Add `devrelay device show`.
- [ ] Add device identity tests.

### M3.4 Session Model

- [ ] Add session ID generation.
- [ ] Add default session creation on project add.
- [ ] Link session to project.
- [ ] Store session name.
- [ ] Store parent session ID.
- [ ] Store archived timestamp.
- [ ] Add `devrelay sessions list`.
- [ ] Add `devrelay session show`.
- [ ] Add `devrelay session fork`.
- [ ] Add `devrelay session archive`.
- [ ] Add session tests.

### M3.5 Lease State Machine

- [ ] Define lease states as enum.
- [ ] Add active state.
- [ ] Add handoff pending state.
- [ ] Add committing state.
- [ ] Add inactive state.
- [ ] Add forked state.
- [ ] Add archived state.
- [ ] Add epoch field.
- [ ] Add holder device ID field.
- [ ] Add latest snapshot ID field.
- [ ] Add handoff ID field.
- [ ] Add state transition validation.
- [ ] Add illegal transition tests.
- [ ] Add epoch monotonicity tests.

### M3.6 Canonical Publish

- [ ] Add publish transaction.
- [ ] Persist snapshot metadata.
- [ ] Verify session ID.
- [ ] Verify lease epoch.
- [ ] Verify holder device ID.
- [ ] Verify lease state active.
- [ ] Compare-and-swap latest snapshot ID.
- [ ] Preserve stale snapshot without making it latest.
- [ ] Return stale publish error/safe action.
- [ ] Add stale epoch test.
- [ ] Add wrong holder test.
- [ ] Add inactive holder test.
- [ ] Add concurrent publish test.

### M3.7 Handoff Protocol

- [ ] Add handoff ID generation.
- [ ] Add begin handoff transaction.
- [ ] Store expected epoch.
- [ ] Store source device ID.
- [ ] Store target device ID.
- [ ] Store source generation.
- [ ] Store expiration.
- [ ] Add target prepare state.
- [ ] Add target verified state.
- [ ] Add source ready state.
- [ ] Add commit state.
- [ ] Add abort state.
- [ ] Increment lease epoch on commit.
- [ ] Change holder on commit.
- [ ] Reject commit if source generation changed.
- [ ] Reject commit if handoff expired.
- [ ] Add handoff happy path test.
- [ ] Add source-change abort test.
- [ ] Add target-apply-failure test.
- [ ] Add concurrent handoff test.

### M3.8 Inactive Edit Fork

- [ ] Detect publish attempt from inactive workspace.
- [ ] Create fork session.
- [ ] Preserve inactive changes as snapshot.
- [ ] Pin fork snapshot by default.
- [ ] Emit `session.diverged`.
- [ ] Add CLI output for "separate work".
- [ ] Add test that canonical latest is unchanged.
- [ ] Add test that fork snapshot is recoverable.

### M3.9 Crash Recovery

- [ ] Add handoff journal table or records.
- [ ] Record begin handoff.
- [ ] Record target prepare.
- [ ] Record target apply.
- [ ] Record target verified.
- [ ] Record lease committed.
- [ ] Resume incomplete handoff safely.
- [ ] Abort expired incomplete handoff safely.
- [ ] Add crash-before-commit test.
- [ ] Add crash-after-commit test.

### M3 Exit Gate

- [ ] Stale lease publish cannot advance canonical latest.
- [ ] Concurrent handoff attempts resolve deterministically.
- [ ] Target dirty protection happens before lease transfer.
- [ ] Crash between apply and lease commit is recoverable.
- [ ] Inactive edit becomes fork, not canonical change.

## M4. LAN Pairing And Secure Control Plane

### M4.1 Fabric Identity

- [ ] Select crypto crates.
- [ ] Generate fabric root key.
- [ ] Store fabric root key securely in dev mode.
- [ ] Generate device signing key.
- [ ] Generate network certificate key.
- [ ] Store device public identity.
- [ ] Store root public identity.
- [ ] Add recovery export placeholder.
- [ ] Add identity rotation design note.
- [ ] Add identity serialization tests.

### M4.2 Pairing Protocol

- [ ] Define pairing session schema.
- [ ] Generate ephemeral pairing key.
- [ ] Start pairing session on new device.
- [ ] Discover anchor or accept manual address.
- [ ] Compute handshake transcript.
- [ ] Derive short authentication string.
- [ ] Show code on both devices.
- [ ] Require user confirmation.
- [ ] Issue device certificate.
- [ ] Persist paired device.
- [ ] Expire old pairing sessions.
- [ ] Add pairing abort.
- [ ] Add pairing replay test.
- [ ] Add mismatched-code test.

### M4.3 mDNS Discovery

- [ ] Choose mDNS crate.
- [ ] Advertise `_devrelay-anchor._tcp.local`.
- [ ] Advertise `_devrelay-peer._tcp.local`.
- [ ] Include `protocol=1`.
- [ ] Include truncated fabric hint.
- [ ] Include public device ID.
- [ ] Include port.
- [ ] Exclude project names.
- [ ] Exclude repository paths.
- [ ] Exclude usernames.
- [ ] Add discovery browser.
- [ ] Add manual address fallback.
- [ ] Add mDNS disable config.
- [ ] Add privacy test for TXT records.

### M4.4 mTLS Transport

- [ ] Add rustls server config.
- [ ] Add rustls client config.
- [ ] Require TLS for control channel.
- [ ] Validate device certificate.
- [ ] Pin fabric root.
- [ ] Check revocation denylist.
- [ ] Add protocol version negotiation.
- [ ] Add request timestamp.
- [ ] Add replay nonce.
- [ ] Add max clock skew policy.
- [ ] Add connection timeout.
- [ ] Add request timeout.
- [ ] Add expired cert test.
- [ ] Add revoked cert test.
- [ ] Add wrong fabric test.

### M4.5 Control API

- [ ] Add HTTP server or RPC server over TLS.
- [ ] Add `/v1/devices`.
- [ ] Add `/v1/projects`.
- [ ] Add `/v1/workspaces`.
- [ ] Add `/v1/sessions/{id}/snapshots`.
- [ ] Add `/v1/handoffs`.
- [ ] Add `/v1/recovery`.
- [ ] Add API auth middleware.
- [ ] Add request ID middleware.
- [ ] Add JSON error mapping.
- [ ] Add OpenAPI or schema documentation.
- [ ] Add API integration tests.

### M4.6 Revocation

- [ ] Add device revoke command.
- [ ] Add revocation record.
- [ ] Reject revoked device connection.
- [ ] Reject revoked publish.
- [ ] Reject revoked lease operation.
- [ ] Emit audit event.
- [ ] Add key rotation option placeholder.
- [ ] Add revoke tests.

### M4.7 Audit Log

- [ ] Record pair event.
- [ ] Record revoke event.
- [ ] Record snapshot publish event.
- [ ] Record snapshot apply event.
- [ ] Record lease transfer event.
- [ ] Record command approval event.
- [ ] Record security block event.
- [ ] Add audit query CLI.
- [ ] Add audit export with redaction.
- [ ] Add audit tests.

### M4 Exit Gate

- [ ] Pairing requires matching displayed code.
- [ ] Revoked device cannot connect.
- [ ] mDNS TXT records do not leak sensitive data.
- [ ] Transport tests cover expired, wrong-fabric, and revoked certificates.
- [ ] Control API rejects unauthenticated requests.

## M5. Data Plane: Git Object Transfer And Sidecar CAS

### M5.1 Git Object Data Plane

- [ ] Choose first implementation strategy.
- [ ] Define allowed ref namespace.
- [ ] Serve project snapshot bare repo.
- [ ] Restrict fetch to `refs/devrelay/*`.
- [ ] Restrict push to permitted snapshot refs.
- [ ] Enforce project authorization.
- [ ] Enforce object size limits.
- [ ] Enforce repository quota.
- [ ] Add object availability check.
- [ ] Add object corruption detection.
- [ ] Add data-plane integration tests.

### M5.2 Anchor Snapshot Repos

- [ ] Create anchor project repo on project registration.
- [ ] Store snapshot refs in anchor repo.
- [ ] Fetch source snapshot into anchor repo.
- [ ] Serve target fetch from anchor repo.
- [ ] Add orphan snapshot scan.
- [ ] Add anchor repo maintenance command.
- [ ] Add anchor repo GC guard.
- [ ] Add anchor repo tests.

### M5.3 Route Selection

- [ ] Measure source online status.
- [ ] Measure anchor availability.
- [ ] Add direct peer route.
- [ ] Add anchor cache route.
- [ ] Add source-required route.
- [ ] Add route decision explanation.
- [ ] Add route fallback on failure.
- [ ] Add route metrics.
- [ ] Add direct route tests.
- [ ] Add anchor fallback tests.

### M5.4 CAS Core

- [ ] Define chunk hash format.
- [ ] Define CAS manifest schema.
- [ ] Add chunk missing query.
- [ ] Add chunk upload endpoint.
- [ ] Add chunk download endpoint.
- [ ] Verify chunk hash on upload.
- [ ] Verify chunk hash on download.
- [ ] Store chunk atomically.
- [ ] Deduplicate chunks.
- [ ] Add manifest create endpoint.
- [ ] Add manifest fetch endpoint.
- [ ] Add CAS reachability root concept.
- [ ] Add CAS tests.

### M5.5 Large Sidecars

- [ ] Detect untracked file over threshold.
- [ ] Store large accepted file in CAS.
- [ ] Add content-defined chunking placeholder or fixed chunk first pass.
- [ ] Bound upload buffer memory.
- [ ] Bound download buffer memory.
- [ ] Add sidecar root hash.
- [ ] Add sidecar file mode.
- [ ] Add sidecar logical path.
- [ ] Add sidecar classification.
- [ ] Add sidecar manifest to snapshot metadata.
- [ ] Add large file round-trip test.
- [ ] Add corrupt chunk test.
- [ ] Add missing chunk test.

### M5.6 Sidecar Materialization

- [ ] Fetch required CAS manifest before apply.
- [ ] Query missing chunks.
- [ ] Download missing chunks.
- [ ] Verify chunks.
- [ ] Materialize sidecar file atomically.
- [ ] Restore file mode where supported.
- [ ] Prevent path traversal.
- [ ] Prevent symlink escape.
- [ ] Verify materialized root hash.
- [ ] Add materialization tests.

### M5.7 Partial Upload Safety

- [ ] Mark snapshot data upload as pending.
- [ ] Upload Git objects before metadata publish.
- [ ] Upload CAS chunks before metadata publish.
- [ ] Verify anchor has required data.
- [ ] Publish metadata only after data availability.
- [ ] Ensure partial upload does not update latest.
- [ ] Add network cut fault test.
- [ ] Add upload retry test.
- [ ] Add orphan cleanup test.

### M5 Exit Gate

- [ ] Large sidecar transfer uses bounded memory.
- [ ] Missing chunk blocks apply before lease transfer.
- [ ] Anchor can serve snapshot after source offline.
- [ ] Partial upload never changes canonical latest.
- [ ] Data plane enforces project authorization.

## M6. Background Protection

### M6.1 Filesystem Watcher

- [ ] Define watcher trait.
- [ ] Implement macOS watcher.
- [ ] Implement Linux watcher.
- [ ] Implement Windows watcher.
- [ ] Add polling fallback for unsupported platforms.
- [ ] Treat events as hints only.
- [ ] Increment source generation on relevant events.
- [ ] Coalesce path sets.
- [ ] Drop events outside registered workspaces.
- [ ] Add watcher lifecycle management.
- [ ] Add watcher tests with synthetic events.

### M6.2 Adaptive Debounce

- [ ] Add first-event quiet timer.
- [ ] Add minimum checkpoint interval.
- [ ] Add max dirty interval.
- [ ] Add publish quiet timer.
- [ ] Add max publish interval.
- [ ] Add immediate flush on explicit checkpoint.
- [ ] Add immediate flush on handoff.
- [ ] Add immediate flush on sleep/lock signal where available.
- [ ] Add debounce tests.
- [ ] Add coalescing tests.

### M6.3 Background Checkpoint

- [ ] Track dirty workspace state.
- [ ] Trigger Git status after quiet window.
- [ ] Skip checkpoint if semantic state unchanged.
- [ ] Create local snapshot.
- [ ] Publish to anchor if available.
- [ ] Emit protection status event.
- [ ] Avoid notifications for normal success.
- [ ] Surface repeated failures.
- [ ] Add background checkpoint tests.

### M6.4 Git Performance Doctor

- [ ] Detect Git version.
- [ ] Detect FSMonitor support.
- [ ] Detect existing FSMonitor config.
- [ ] Detect untracked cache support.
- [ ] Detect existing untracked cache config.
- [ ] Add safe recommendation output.
- [ ] Add `doctor --fix-safe` for approved config only.
- [ ] Avoid overwriting user-managed config.
- [ ] Add doctor tests.

### M6.5 Resource Policy

- [ ] Define adaptive profile.
- [ ] Define instant profile.
- [ ] Define eco profile.
- [ ] Define custom profile.
- [ ] Add CPU slot limit.
- [ ] Add hashing concurrency limit.
- [ ] Add network bandwidth cap.
- [ ] Add battery mode behavior.
- [ ] Add low-power mode behavior.
- [ ] Add foreground load detection.
- [ ] Add resource policy persistence.
- [ ] Add resource policy tests.

### M6.6 Retention And Quota

- [ ] Define hot snapshot retention.
- [ ] Define hourly thinning.
- [ ] Define daily thinning.
- [ ] Protect latest canonical snapshot.
- [ ] Protect pinned snapshots.
- [ ] Protect handoff snapshots for configured duration.
- [ ] Add device cache quota.
- [ ] Add anchor project quota.
- [ ] Add free disk warning threshold.
- [ ] Add free disk hard stop threshold.
- [ ] Add pruning planner.
- [ ] Add pruning executor.
- [ ] Add retention tests.
- [ ] Add quota tests.

### M6.7 Crash Journal

- [ ] Add journal record type.
- [ ] Record snapshot creation start.
- [ ] Record snapshot creation complete.
- [ ] Record publish start.
- [ ] Record publish complete.
- [ ] Record target apply start.
- [ ] Record target backup complete.
- [ ] Record base applied.
- [ ] Record work applied.
- [ ] Record index applied.
- [ ] Record verified.
- [ ] Record lease committed.
- [ ] Add journal replay.
- [ ] Add journal cleanup.
- [ ] Add fault injection tests.

### M6 Exit Gate

- [ ] Idle agent CPU/RSS meets target on test repos.
- [ ] Many file events coalesce into bounded work.
- [ ] Disk pressure prunes only unpinned/evictable data.
- [ ] Background failures surface as protection status.
- [ ] Background watcher is not used as source of truth.

## M7. Desktop UX: Tray And Dashboard

### M7.1 Tauri Shell

- [ ] Create desktop app package.
- [ ] Wire Rust backend to local agent client.
- [ ] Add frontend build setup.
- [ ] Import visual direction from prototype.
- [ ] Add dark theme.
- [ ] Add light theme.
- [ ] Add responsive layout.
- [ ] Add keyboard navigation baseline.
- [ ] Add reduced motion mode.
- [ ] Add accessibility labels.
- [ ] Add app icon placeholder.
- [ ] Add development run script.

### M7.2 Agent State Binding

- [ ] Subscribe to agent event stream.
- [ ] Load initial status from agent.
- [ ] Keep UI state read-only from agent events.
- [ ] Add reconnection state.
- [ ] Add offline agent state.
- [ ] Add stale data indicator.
- [ ] Add event gap recovery.
- [ ] Add frontend state tests.

### M7.3 Continue View

- [ ] Show current device.
- [ ] Show most likely continuation session.
- [ ] Show active writer.
- [ ] Show latest checkpoint age.
- [ ] Show Git state summary.
- [ ] Show target readiness.
- [ ] Show environment warmth.
- [ ] Add primary "Continue here" action.
- [ ] Add "Continue elsewhere" action.
- [ ] Add "Checkpoint" action.
- [ ] Add "Run elsewhere" action placeholder.
- [ ] Add empty state.
- [ ] Add error state.
- [ ] Add loading state.

### M7.4 Projects View

- [ ] List registered projects.
- [ ] Show active session per project.
- [ ] Show writer device.
- [ ] Show checkpoint status.
- [ ] Show availability per target.
- [ ] Add project filter.
- [ ] Add project add action.
- [ ] Add project detail link.
- [ ] Add needs-attention grouping.
- [ ] Add recovery entry point.

### M7.5 Devices View

- [ ] List paired devices.
- [ ] Show online/offline state.
- [ ] Show OS and architecture.
- [ ] Show role capabilities.
- [ ] Show CPU summary.
- [ ] Show memory summary.
- [ ] Show disk summary.
- [ ] Show battery/AC state.
- [ ] Show cache warmth.
- [ ] Add revoke action placeholder.
- [ ] Add pair device action placeholder.

### M7.6 Handoff Dialog

- [ ] Show source device.
- [ ] Show target device.
- [ ] Show project/session.
- [ ] Show checkpoint age.
- [ ] Show staged count.
- [ ] Show unstaged count.
- [ ] Show untracked count.
- [ ] Show unpushed commit count when available.
- [ ] Show environment readiness.
- [ ] Show editor context readiness.
- [ ] Show dirty target safe actions.
- [ ] Show progress phase "saving state".
- [ ] Show progress phase "preparing device".
- [ ] Show progress phase "moving control".
- [ ] Handle abort.
- [ ] Handle failure.
- [ ] Handle success.

### M7.7 Runs View

- [ ] Show recent runs.
- [ ] Show queued runs.
- [ ] Show running runs.
- [ ] Show failed runs.
- [ ] Show scheduler explanation.
- [ ] Add run task action placeholder.
- [ ] Add cancel action placeholder.
- [ ] Add artifact action placeholder.

### M7.8 Activity View

- [ ] Show audit events.
- [ ] Show checkpoint events summarized.
- [ ] Show handoff events.
- [ ] Show security blocks.
- [ ] Show quota warnings.
- [ ] Add filters.
- [ ] Add diagnostic bundle export action.

### M7.9 Settings View

- [ ] Add background behavior settings.
- [ ] Add storage/cache settings.
- [ ] Add network settings.
- [ ] Add security settings.
- [ ] Add editor context settings.
- [ ] Add advanced diagnostics settings.
- [ ] Save settings through agent.
- [ ] Validate settings before save.

### M7.10 Tray/Menu Bar

- [ ] Add tray icon.
- [ ] Show active project.
- [ ] Show latest checkpoint age.
- [ ] Show protection state.
- [ ] Add continue target list.
- [ ] Add run elsewhere shortcuts.
- [ ] Add open dashboard.
- [ ] Add pause background work.
- [ ] Add quit.
- [ ] Add two-click handoff path.

### M7 Exit Gate

- [ ] Same-LAN handoff can start from tray in two clicks.
- [ ] Critical state is not color-only.
- [ ] Dirty target flow avoids Git jargon.
- [ ] UI state comes from agent events.
- [ ] Keyboard-only core flow works.
- [ ] Screen reader labels cover primary actions.

## M8. Editor Context: VS Code First

### M8.1 Extension Skeleton

- [ ] Create VS Code extension package.
- [ ] Add TypeScript build.
- [ ] Add linting.
- [ ] Add extension activation events.
- [ ] Add local agent client.
- [ ] Add connection status.
- [ ] Add command registration.
- [ ] Add extension tests.

### M8.2 Workspace Context Capture

- [ ] Capture workspace folder.
- [ ] Capture opened files.
- [ ] Capture tab order where API allows.
- [ ] Capture active editor.
- [ ] Capture cursor position.
- [ ] Capture selections.
- [ ] Capture split layout where API allows.
- [ ] Capture breakpoints where API allows.
- [ ] Capture terminal cwd where API allows.
- [ ] Capture terminal title where API allows.
- [ ] Send context capsule to agent.
- [ ] Add context size limits.
- [ ] Add capture tests where practical.

### M8.3 Unsaved Buffers

- [ ] Detect unsaved dirty documents.
- [ ] Capture unsaved text content.
- [ ] Exclude untitled buffers unless user permits.
- [ ] Encrypt buffer capsule placeholder or require local-only first pass.
- [ ] Store buffer capsule separately from Git snapshot.
- [ ] Restore buffer as dirty document.
- [ ] Never write restored unsaved buffer to disk automatically.
- [ ] Add user setting to disable unsaved buffer capture.
- [ ] Add tests for dirty restore behavior.

### M8.4 Active/Inactive Indicator

- [ ] Show active writer state in status bar.
- [ ] Show inactive workspace warning.
- [ ] Show handoff-in-progress state.
- [ ] Show protection delayed state.
- [ ] Add command to explain state.
- [ ] Add command to open dashboard.

### M8.5 Handoff Edit Guard

- [ ] Notify agent when text document changes.
- [ ] Notify agent when file save occurs.
- [ ] Notify agent when active editor changes.
- [ ] Increment source generation on meaningful edit.
- [ ] Show handoff-in-progress warning.
- [ ] Abort handoff on source generation change.
- [ ] Add tests with mocked agent.

### M8.6 Context Restore

- [ ] Receive restore command from agent.
- [ ] Open workspace folder.
- [ ] Open files in saved order.
- [ ] Restore active file.
- [ ] Restore cursor positions.
- [ ] Restore selections.
- [ ] Restore split layout where possible.
- [ ] Restore breakpoints where possible.
- [ ] Restore unsaved buffers as dirty.
- [ ] Report restore ACK to agent.
- [ ] Report partial restore details.
- [ ] Add restore tests where possible.

### M8.7 Editor Commands

- [ ] Add "DevRelay: Continue Here".
- [ ] Add "DevRelay: Continue Elsewhere".
- [ ] Add "DevRelay: Checkpoint".
- [ ] Add "DevRelay: Run Task".
- [ ] Add "DevRelay: Open Recovery Timeline".
- [ ] Add "DevRelay: Explain Current State".
- [ ] Add command palette metadata.

### M8 Exit Gate

- [ ] Handoff opens target editor with expected context.
- [ ] Unsaved buffers restore as dirty buffers.
- [ ] Extension failure does not block verified code handoff.
- [ ] Source edit during handoff prevents stale lease transfer.
- [ ] User can disable editor context capture.

## M9. Environment Hydration

### M9.1 Trust Hashes

- [ ] Define canonical command hash algorithm.
- [ ] Hash environment command.
- [ ] Hash healthcheck command.
- [ ] Hash task command.
- [ ] Hash bootstrap script content.
- [ ] Store approved hashes per project/device.
- [ ] Detect changed command hash.
- [ ] Add "allow once" approval.
- [ ] Add "trust this version" approval.
- [ ] Add "reject" path.
- [ ] Add trust hash tests.

### M9.2 Profile Selection

- [ ] Detect current platform key.
- [ ] Match manifest profile targets.
- [ ] Prefer Nix when compatible.
- [ ] Prefer Dev Container after Nix.
- [ ] Prefer native declarative tool manager after Dev Container.
- [ ] Use trusted bootstrap script after declarative options.
- [ ] Fall back to manual profile.
- [ ] Explain selected profile.
- [ ] Add profile selection tests.

### M9.3 Nix Adapter

- [ ] Detect `nix` availability.
- [ ] Detect flake files.
- [ ] Compute flake fingerprint.
- [ ] Run `nix develop` health check flow.
- [ ] Capture shell-ready state.
- [ ] Capture failure logs.
- [ ] Add store path prefetch placeholder.
- [ ] Add LAN binary cache config placeholder.
- [ ] Add platform-specific cache warmth.
- [ ] Add Nix adapter tests with mocked commands.

### M9.4 Dev Container Adapter

- [ ] Detect `.devcontainer/devcontainer.json`.
- [ ] Detect container engine availability.
- [ ] Compute devcontainer fingerprint.
- [ ] Pull/build image with user approval.
- [ ] Prepare isolated workspace mount.
- [ ] Run health check.
- [ ] Capture logs.
- [ ] Add adapter tests with mocked commands.

### M9.5 Native Bootstrap Adapter

- [ ] Support PowerShell command profile.
- [ ] Support POSIX shell command profile.
- [ ] Require trust approval before execution.
- [ ] Enforce timeout.
- [ ] Capture stdout/stderr with redaction.
- [ ] Store bootstrap result.
- [ ] Run health check.
- [ ] Add idempotency warning in docs.
- [ ] Add native adapter tests with mocked commands.

### M9.6 Secret Providers

- [ ] Define secret mapping local config.
- [ ] Add OS keychain provider design.
- [ ] Add 1Password CLI provider.
- [ ] Add Bitwarden CLI provider.
- [ ] Add SOPS/age provider.
- [ ] Add user script provider.
- [ ] Materialize secret file locally.
- [ ] Materialize environment variable locally.
- [ ] Add secret path to hard exclude.
- [ ] Redact secret values from logs.
- [ ] Add missing required secret error.
- [ ] Add secret provider tests with fake provider.

### M9.7 Hydration State Machine

- [ ] Add cold state.
- [ ] Add metadata-ready state.
- [ ] Add cache-ready state.
- [ ] Add shell-ready state.
- [ ] Add app-ready state.
- [ ] Add failed state.
- [ ] Add retry transition.
- [ ] Add state persistence.
- [ ] Emit environment progress events.
- [ ] Add state machine tests.

### M9.8 Environment Doctor

- [ ] Detect missing Nix.
- [ ] Detect missing container engine.
- [ ] Detect missing PowerShell.
- [ ] Detect changed command hash.
- [ ] Detect missing required secret.
- [ ] Detect incompatible platform target.
- [ ] Detect healthcheck failure.
- [ ] Add suggested safe action for each error.
- [ ] Add `devrelay doctor environment`.
- [ ] Add doctor tests.

### M9 Exit Gate

- [ ] Manifest command changes require explicit trust.
- [ ] Required secrets materialize locally and remain excluded from snapshots.
- [ ] Environment failure leaves code state intact.
- [ ] Warm target can enter dev shell within SLO on representative project.
- [ ] Hydration state is visible in CLI and UI.

## M10. Compute Fabric

### M10.1 Task Model

- [ ] Parse task definitions from manifest.
- [ ] Validate task profile exists.
- [ ] Validate task command.
- [ ] Validate task platform constraints.
- [ ] Validate resource hints.
- [ ] Validate sandbox setting.
- [ ] Compute command definition hash.
- [ ] Create immutable execution snapshot.
- [ ] Store task run metadata.
- [ ] Add task model tests.

### M10.2 Scheduler Constraints

- [ ] Collect static device OS.
- [ ] Collect static architecture.
- [ ] Collect CPU core count.
- [ ] Collect memory capacity.
- [ ] Collect disk capacity.
- [ ] Collect dynamic CPU load.
- [ ] Collect dynamic free memory.
- [ ] Collect dynamic free disk.
- [ ] Collect AC/battery state.
- [ ] Collect foreground activity.
- [ ] Collect network route quality placeholder.
- [ ] Filter incompatible platforms.
- [ ] Filter missing features.
- [ ] Filter insufficient memory.
- [ ] Filter insufficient disk.
- [ ] Filter disallowed policy.
- [ ] Add constraints tests.

### M10.3 Scheduler Score

- [ ] Implement cache warmth score.
- [ ] Implement idle CPU score.
- [ ] Implement free memory score.
- [ ] Implement power preference score.
- [ ] Implement data locality score.
- [ ] Implement network quality score.
- [ ] Implement historical speed score.
- [ ] Implement user affinity score.
- [ ] Implement transfer cost penalty.
- [ ] Implement foreground penalty.
- [ ] Implement thermal penalty placeholder.
- [ ] Add task-class weight profiles.
- [ ] Add explanation output.
- [ ] Add scheduler score tests.

### M10.4 Runner Workspace

- [ ] Create isolated task workspace.
- [ ] Apply execution snapshot.
- [ ] Hydrate environment.
- [ ] Materialize required sidecars.
- [ ] Materialize required secrets only when permitted.
- [ ] Prevent task workspace from being canonical session.
- [ ] Clean up workspace by retention policy.
- [ ] Add runner workspace tests.

### M10.5 Runner Execution

- [ ] Run host task.
- [ ] Add sandbox placeholder.
- [ ] Add container runner placeholder.
- [ ] Add VM runner placeholder.
- [ ] Set working directory.
- [ ] Set environment variables.
- [ ] Enforce timeout.
- [ ] Capture exit code.
- [ ] Capture stdout/stderr.
- [ ] Stream logs live.
- [ ] Cancel process tree.
- [ ] Add execution tests with fake commands.

### M10.6 Logs

- [ ] Add bounded live log buffer.
- [ ] Add disk spool.
- [ ] Add log redaction.
- [ ] Add log event streaming.
- [ ] Add log retrieval API.
- [ ] Add log truncation marker.
- [ ] Add log tests.

### M10.7 Artifacts

- [ ] Parse declared outputs.
- [ ] Capture output files.
- [ ] Reject output path traversal.
- [ ] Hash artifacts.
- [ ] Store artifacts in CAS.
- [ ] Store artifact index.
- [ ] Return summary first.
- [ ] Add on-demand artifact pull.
- [ ] Add artifact retention.
- [ ] Add artifact tests.

### M10.8 Result Cache

- [ ] Define task cache key.
- [ ] Include input snapshot tree.
- [ ] Include declared sidecar inputs.
- [ ] Include environment fingerprint.
- [ ] Include command definition hash.
- [ ] Include platform.
- [ ] Exclude secret values.
- [ ] Disable cache for secret-sensitive tasks by default.
- [ ] Store cache result.
- [ ] Return cache hit.
- [ ] Add cache tests.

### M10.9 Nix Delegation

- [ ] Detect Nix task.
- [ ] Generate temporary builder set.
- [ ] Respect device capabilities.
- [ ] Integrate remote builder logs.
- [ ] Publish result to LAN binary cache.
- [ ] Explain delegation decision.
- [ ] Add mocked Nix delegation tests.

### M10.10 Code-Changing Agent Task

- [ ] Mark task as code-changing.
- [ ] Create separate session.
- [ ] Run in isolated workspace.
- [ ] Capture commit chain or diff.
- [ ] Run declared tests.
- [ ] Return summary.
- [ ] Show changed files.
- [ ] Never auto-merge into active session.
- [ ] Add tests proving canonical session unchanged.

### M10 Exit Gate

- [ ] Remote task cannot mutate canonical active session.
- [ ] Scheduler explains chosen target.
- [ ] Cancellation kills process tree.
- [ ] Artifact capture cannot escape declared outputs.
- [ ] Cache key excludes secret values.

## M11. Cross-Platform Hardening

### M11.1 Platform Identity

- [ ] Define platform key format.
- [ ] Detect Linux GNU x86_64.
- [ ] Detect Linux aarch64.
- [ ] Detect Darwin arm64.
- [ ] Detect Darwin x86_64.
- [ ] Detect Windows native x86_64.
- [ ] Detect WSL2 Linux.
- [ ] Detect WSL distro.
- [ ] Detect WSL version.
- [ ] Store platform capabilities.
- [ ] Add platform detection tests.

### M11.2 Path Portability Doctor

- [ ] Walk Git-tracked paths.
- [ ] Walk accepted untracked paths.
- [ ] Detect case-fold collisions.
- [ ] Detect Unicode normalization collisions.
- [ ] Detect Windows reserved names.
- [ ] Detect trailing dot.
- [ ] Detect trailing space.
- [ ] Detect invalid Windows characters.
- [ ] Detect path length budget violations.
- [ ] Detect symlink capability mismatch.
- [ ] Report path conflict codes.
- [ ] Add target exclusion safe action.
- [ ] Add rename guidance safe action.
- [ ] Add portability tests.

### M11.3 Line Endings

- [ ] Read `.gitattributes`.
- [ ] Detect missing policy.
- [ ] Detect conflicting `core.autocrlf`.
- [ ] Verify by Git semantic hash.
- [ ] Add warning for risky target config.
- [ ] Add line-ending tests.

### M11.4 Executable Bit

- [ ] Preserve Git tree mode.
- [ ] Verify executable bit on POSIX.
- [ ] Preserve index mode on Windows.
- [ ] Add executable-bit tests.

### M11.5 Symlink Policy

- [ ] Detect symlink entries.
- [ ] Preserve symlink target string.
- [ ] Refuse to follow symlink outside workspace.
- [ ] Block materialization if target lacks symlink support.
- [ ] Add symlink capability doctor.
- [ ] Add symlink tests.

### M11.6 Windows Reparse Points

- [ ] Detect reparse points in workspace.
- [ ] Prevent reparse point traversal during scan.
- [ ] Prevent reparse point traversal during materialization.
- [ ] Add Windows-specific tests where possible.

### M11.7 WSL/Native Separation

- [ ] Represent Windows native as separate device.
- [ ] Represent WSL distro as separate device.
- [ ] Prevent shared tree mutation warning.
- [ ] Add workspace mapping guidance.
- [ ] Add WSL filesystem doctor.
- [ ] Add WSL tests where possible.

### M11 Exit Gate

- [ ] Unsafe paths block before target apply.
- [ ] Windows native and WSL are separate devices.
- [ ] Symlink/reparse traversal tests pass.
- [ ] Line-ending verification uses Git semantics.

## M12. Advanced Git States

### M12.1 Merge/Cherry-Pick/Revert Conflicts

- [ ] Detect merge conflict state.
- [ ] Detect cherry-pick conflict state.
- [ ] Detect revert conflict state.
- [ ] Capture index stage 1 entries.
- [ ] Capture index stage 2 entries.
- [ ] Capture index stage 3 entries.
- [ ] Capture operation metadata.
- [ ] Store operation capsule.
- [ ] Apply unmerged index to target.
- [ ] Restore conflict markers.
- [ ] Verify staged entries.
- [ ] Add conflict round-trip tests.

### M12.2 Submodules

- [ ] Detect submodule config.
- [ ] Detect clean submodule state.
- [ ] Restore clean submodule recorded commit.
- [ ] Detect dirty submodule state.
- [ ] Create child project/session for dirty submodule.
- [ ] Store parent-child snapshot topology.
- [ ] Add recursion depth limit.
- [ ] Add cycle detection.
- [ ] Add submodule tests.

### M12.3 Git LFS

- [ ] Detect LFS pointer files.
- [ ] Detect required local LFS objects.
- [ ] Check upstream availability.
- [ ] Store local-only LFS object in CAS.
- [ ] Verify LFS object before apply.
- [ ] Block apply if object missing.
- [ ] Add LFS tests with fake objects.

### M12.4 Sparse Checkout And Partial Clone

- [ ] Detect sparse checkout.
- [ ] Capture sparse definition as workspace preference.
- [ ] Distinguish logical snapshot from sparse view.
- [ ] Fetch missing blobs on demand.
- [ ] Avoid overwriting target sparse policy.
- [ ] Add sparse checkout tests.
- [ ] Add partial clone tests where practical.

### M12.5 Interactive Rebase And Sequencer

- [ ] Detect interactive rebase.
- [ ] Detect sequencer state.
- [ ] Capture original HEAD.
- [ ] Capture onto commit.
- [ ] Capture todo list.
- [ ] Capture current step.
- [ ] Define target Git version compatibility.
- [ ] Reconstruct operation on target.
- [ ] Add safe block fallback.
- [ ] Add operation capsule tests.
- [ ] Keep feature disabled until tests are exhaustive.

### M12 Exit Gate

- [ ] Conflict round-trip preserves staged entries.
- [ ] Unsupported operations block with recovery options.
- [ ] Rebase support remains gated until exhaustive tests pass.
- [ ] Advanced states never silently degrade into normal snapshots.

## M13. Security, Privacy, And Operations Gate

### M13.1 Threat Model

- [ ] Document same-LAN attacker.
- [ ] Document malicious project manifest.
- [ ] Document stale device.
- [ ] Document compromised device.
- [ ] Document path traversal.
- [ ] Document symlink escape.
- [ ] Document secret leakage.
- [ ] Document replay attack.
- [ ] Document data corruption.
- [ ] Document accepted trust assumptions.
- [ ] Map mitigations to each threat.
- [ ] Review threat model with implementation.

### M13.2 Fuzzing

- [ ] Add fuzz target for porcelain parser.
- [ ] Add fuzz target for manifest parser.
- [ ] Add fuzz target for path canonicalization.
- [ ] Add fuzz target for CAS manifest parser.
- [ ] Add fuzz target for network API payloads.
- [ ] Add fuzz corpus seeds.
- [ ] Add fuzz run documentation.
- [ ] Add CI/nightly fuzz plan.

### M13.3 Secret Scanning

- [ ] Expand sensitive filename rules.
- [ ] Add private key header scanner.
- [ ] Add token pattern scanner.
- [ ] Add high-entropy heuristic.
- [ ] Add user-configured scanner hook.
- [ ] Add raw secret redaction tests.
- [ ] Add false-positive override design.
- [ ] Add encrypted one-time sidecar design.

### M13.4 Update And Migration

- [ ] Define release channel model.
- [ ] Define signed release strategy.
- [ ] Define binary provenance strategy.
- [ ] Define metadata schema migration support window.
- [ ] Define snapshot schema migration support window.
- [ ] Add rollback expectations.
- [ ] Add migration tests from old fixtures.

### M13.5 Backup Anchor

- [ ] Define backup anchor data set.
- [ ] Replicate metadata DB safely.
- [ ] Replicate snapshot bare repos.
- [ ] Replicate CAS chunks.
- [ ] Sign replicated state.
- [ ] Add manual promotion command.
- [ ] Add promotion audit event.
- [ ] Add backup restore test.

### M13.6 Opaque Anchor Research

- [ ] Document encrypted bundle design.
- [ ] Document encrypted chunk design.
- [ ] Document dedup tradeoff.
- [ ] Document Git negotiation tradeoff.
- [ ] Build proof-of-concept if justified.
- [ ] Decide whether to include in North Star release.

### M13.7 Diagnostic Bundle

- [ ] Include version/capability.
- [ ] Include redacted config.
- [ ] Include recent structured logs.
- [ ] Include state machine records.
- [ ] Include Git command exit codes.
- [ ] Include timing metrics.
- [ ] Exclude source code by default.
- [ ] Exclude snapshot objects by default.
- [ ] Add explicit sensitive export option.
- [ ] Add diagnostic tests.

### M13.8 Fault Injection

- [ ] Inject crash during snapshot object write.
- [ ] Inject crash during ref update.
- [ ] Inject network cut during publish.
- [ ] Inject crash during metadata transaction.
- [ ] Inject crash during target fetch.
- [ ] Inject crash during base apply.
- [ ] Inject crash during work apply.
- [ ] Inject crash during index apply.
- [ ] Inject crash during verification.
- [ ] Inject crash during lease commit.
- [ ] Inject disk-full during CAS upload.
- [ ] Inject disk-full during target apply.
- [ ] Add zero-data-loss assertions.

### M13 Exit Gate

- [ ] Independent security review has no unresolved critical findings.
- [ ] Revocation blocks new access and lease operations.
- [ ] Diagnostic export excludes source/snapshot by default.
- [ ] Fault injection produces zero data loss in supported states.
- [ ] Secret hard-block regression count is zero.

## M14. Beta Product Loop

### M14.1 Installers And Dev Channels

- [ ] Define dev channel.
- [ ] Define beta channel.
- [ ] Package macOS agent.
- [ ] Package macOS desktop app.
- [ ] Package Linux agent.
- [ ] Package Linux desktop app.
- [ ] Package Windows agent.
- [ ] Package Windows desktop app.
- [ ] Package WSL agent instructions.
- [ ] Add upgrade path.
- [ ] Add uninstall path.

### M14.2 Onboarding

- [ ] Build first-device flow.
- [ ] Build anchor selection flow.
- [ ] Build peer-only selection flow.
- [ ] Build existing-anchor connection flow.
- [ ] Build short authentication string confirmation.
- [ ] Build project discovery flow.
- [ ] Build environment detection flow.
- [ ] Build command trust prompt.
- [ ] Build recovery key export prompt.
- [ ] Add onboarding tests.

### M14.3 Guided Doctor

- [ ] Add project safety doctor.
- [ ] Add path portability doctor.
- [ ] Add environment doctor.
- [ ] Add secret mapping doctor.
- [ ] Add resource policy doctor.
- [ ] Add anchor health doctor.
- [ ] Add device trust doctor.
- [ ] Add one-click safe fixes where appropriate.
- [ ] Add doctor UX tests.

### M14.4 Local Metrics

- [ ] Track verified continuation attempts.
- [ ] Track verified continuation successes.
- [ ] Track checkpoint success.
- [ ] Track checkpoint failure reasons.
- [ ] Track apply verification failures.
- [ ] Track handoff phase durations.
- [ ] Track environment hydrate duration.
- [ ] Track scheduler choice reason.
- [ ] Keep metrics local by default.
- [ ] Add metrics export with redaction.
- [ ] Add metrics tests.

### M14.5 User Documentation

- [ ] Write quick start.
- [ ] Write device pairing guide.
- [ ] Write project registration guide.
- [ ] Write handoff guide.
- [ ] Write recovery guide.
- [ ] Write target dirty explanation.
- [ ] Write inactive edit explanation.
- [ ] Write unsupported states guide.
- [ ] Write security model guide.
- [ ] Write backup guide.
- [ ] Write troubleshooting guide.

### M14.6 Dogfooding

- [ ] Select two-device dogfood scenario.
- [ ] Select three-device dogfood scenario.
- [ ] Test macOS to Linux handoff.
- [ ] Test Linux to macOS handoff.
- [ ] Test Windows native to Linux handoff.
- [ ] Test WSL to macOS handoff.
- [ ] Test target dirty scenario.
- [ ] Test inactive edit scenario.
- [ ] Test anchor offline scenario.
- [ ] Test environment cold scenario.
- [ ] Record defects.
- [ ] Feed defects back into checklist.

### M14 Exit Gate

- [ ] One-click handoff works between two real devices.
- [ ] Target dirty flow is safe and understandable.
- [ ] Inactive edit flow is safe and understandable.
- [ ] Normal handoff requires no Git command.
- [ ] Local metrics remain private by default.
- [ ] User docs cover recovery and backup.

## M15. North Star Release Candidate

### M15.1 Required Capabilities

- [ ] Anchor-first topology works.
- [ ] Peer-only mode works with documented limitations.
- [ ] Manual/static discovery fallback works.
- [ ] Pair devices works.
- [ ] Revoke devices works.
- [ ] Project registry works across devices.
- [ ] Workspace mapping works across devices.
- [ ] Single canonical writer lease works.
- [ ] Explicit checkpoint works.
- [ ] Background checkpoint works.
- [ ] Direct snapshot transfer works.
- [ ] Anchor snapshot transfer works.
- [ ] Dirty target recovery works.
- [ ] Inactive fork preservation works.
- [ ] Timeline recovery as new session works.
- [ ] Tauri tray/dashboard works.
- [ ] VS Code context restore works.
- [ ] Nix hydration works.
- [ ] Native bootstrap works.
- [ ] Secret materialization works.
- [ ] Remote task runner works.
- [ ] Scheduler works.
- [ ] Artifact return works.
- [ ] Cross-platform doctor works.
- [ ] Diagnostic bundle works.
- [ ] Audit log works.

### M15.2 Verified Continuation Gate

- [ ] Define measurement harness.
- [ ] Measure prefetched target code-ready time.
- [ ] Achieve p95 under 5 seconds for prefetched target.
- [ ] Measure warm environment shell-ready time.
- [ ] Achieve p95 under 15 seconds for warm environment.
- [ ] Measure supported Git state fidelity.
- [ ] Achieve 100 percent fidelity in supported states.
- [ ] Measure data loss incidents.
- [ ] Achieve 0 data loss incidents.
- [ ] Produce release-candidate report.

### M15.3 Correctness Gate

- [ ] Run high-volume randomized round-trip suite.
- [ ] Run fault injection suite.
- [ ] Verify stale lease canonical update count is 0.
- [ ] Verify secret hard-block regression count is 0.
- [ ] Verify target dirty overwrite count is 0.
- [ ] Verify inactive edit canonical merge count is 0.
- [ ] Produce correctness report.

### M15.4 Resource Gate

- [ ] Measure idle agent CPU.
- [ ] Measure idle agent RSS.
- [ ] Measure active checkpoint CPU burst.
- [ ] Measure active checkpoint RSS.
- [ ] Measure background network cap.
- [ ] Measure battery behavior.
- [ ] Measure low-power behavior.
- [ ] Measure quota/GC behavior.
- [ ] Verify pinned/latest data protection.
- [ ] Produce resource report.

### M15.5 UX Gate

- [ ] Test new user device pairing without docs.
- [ ] Test handoff without Git commands.
- [ ] Test target dirty resolution.
- [ ] Test recovery timeline.
- [ ] Test keyboard-only handoff.
- [ ] Test screen reader labels.
- [ ] Test reduced motion.
- [ ] Test actionable error comprehension.
- [ ] Produce UX report.

### M15.6 Security Gate

- [ ] Verify mTLS is mandatory.
- [ ] Verify snapshot metadata signatures.
- [ ] Verify command trust hash enforcement.
- [ ] Verify revocation enforcement.
- [ ] Verify diagnostic redaction.
- [ ] Verify path traversal defenses.
- [ ] Verify secret scanning defenses.
- [ ] Verify replay defenses.
- [ ] Produce security report.

### M15.7 Release Candidate Operations

- [ ] Freeze supported feature set.
- [ ] Freeze schema versions.
- [ ] Freeze CLI JSON schemas.
- [ ] Freeze control API schemas.
- [ ] Freeze migration plan.
- [ ] Build signed binaries.
- [ ] Build installer artifacts.
- [ ] Build docs site or docs bundle.
- [ ] Produce known issues list.
- [ ] Produce rollback instructions.
- [ ] Tag release candidate.

### M15 Exit Gate

- [ ] All required capabilities pass.
- [ ] Verified continuation gate passes.
- [ ] Correctness gate passes.
- [ ] Resource gate passes.
- [ ] UX gate passes.
- [ ] Security gate passes.
- [ ] Release candidate artifacts are signed and reproducible enough for beta.

## Sequencing Checklist

- [ ] Complete M0 before relying on snapshots elsewhere.
- [ ] Complete M1 before building background protection.
- [ ] Complete M2 before wiring GUI to product state.
- [ ] Complete M3 before real cross-device handoff.
- [ ] Complete M4 before trusting LAN devices.
- [ ] Complete M5 before source-offline continuation.
- [ ] Complete M6 before promising invisible protection.
- [ ] Complete M7 after agent event stream is stable.
- [ ] Complete M8 after code handoff is verified without editor context.
- [ ] Complete M9 before promising environment-ready handoff.
- [ ] Complete M10 after immutable execution snapshots are stable.
- [ ] Complete M11 before broad Windows/WSL support claims.
- [ ] Complete M12 only after normal-state correctness is mature.
- [ ] Complete M13 before beta.
- [ ] Complete M14 before release candidate.
- [ ] Complete M15 before claiming North Star behavior.

## Immediate Next 10 Checklist

- [ ] Add stable error codes and JSON error output to the existing CLI.
- [ ] Add `SnapshotMetadata` schema tests and golden JSON fixtures.
- [ ] Expand Git round-trip fixtures for staged delete.
- [ ] Expand Git round-trip fixtures for unstaged delete.
- [ ] Expand Git round-trip fixtures for binary files.
- [ ] Expand Git round-trip fixtures for executable bit.
- [ ] Expand Git round-trip fixtures for Unicode paths.
- [ ] Add apply journal records to local apply.
- [ ] Create SQLite schema and migrations for local project/session/snapshot data.
- [ ] Add `devrelay project add/list` and local registry config.
- [ ] Move snapshot metadata persistence into the registry.
- [ ] Add recovery timeline CLI.
- [ ] Add dirty target backup snapshot rather than only refusal.
- [ ] Add a minimal agent process.
- [ ] Make CLI call the agent in dev mode.

## Non-Negotiable Safety Checklist

- [ ] No implementation path can silently overwrite target work.
- [ ] No background path performs an automatic merge.
- [ ] No plaintext secret is included in a snapshot by default.
- [ ] No remote command runs without trust hash approval.
- [ ] No UI computes canonical state independently from the agent.
- [ ] No watcher event is treated as the source of truth.
- [ ] No cross-device handoff succeeds before verification passes.
- [ ] No compute task writes directly into the active session.
- [ ] Every destructive cleanup has explicit confirmation or prior snapshot.
- [ ] Every recovery operation defaults to a new session or workspace.
- [ ] Every published snapshot is immutable.
- [ ] Every lease epoch transition is monotonic.
- [ ] Every stale publish preserves data as non-canonical work.
- [ ] Every diagnostic export is redacted by default.
