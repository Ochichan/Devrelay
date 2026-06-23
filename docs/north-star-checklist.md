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
- [x] Publish CI artifacts for failed integration test logs.
- [x] Add minimum Rust version enforcement.
- [x] Add dependency audit tooling.
- [x] Add license/dependency inventory tooling.

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
- [x] Add `recover.open`.
- [x] Add `apply.snapshot`.
- [x] Add `diagnostics.export`.
- [x] Add RPC schema tests.
- [x] Add backwards compatibility policy.

### M2.4 CLI Through Agent

- [x] Add client library for local agent RPC.
- [x] Add CLI global `--direct` escape hatch.
- [x] Add CLI global `--agent-socket`.
- [x] Route `status` through agent.
- [x] Route project commands through agent.
- [x] Route checkpoint through agent.
- [x] Route recover through agent.
- [x] Route apply through agent.
- [x] Preserve CLI JSON output compatibility.
- [x] Add fallback message when agent is unavailable.
- [x] Add integration test with spawned agent.

### M2.5 Event Stream

- [x] Define event envelope.
- [x] Add monotonic event sequence.
- [x] Add event timestamp.
- [x] Add event replay cursor.
- [x] Add `workspace.state.changed`.
- [x] Add `snapshot.local.created`.
- [x] Add `snapshot.apply.started`.
- [x] Add `snapshot.apply.verified`.
- [x] Add `security.blocked`.
- [x] Add `quota.warning`.
- [x] Add subscription RPC.
- [x] Add reconnect behavior.
- [x] Add gap detection.
- [x] Add event stream tests.

### M2.6 Structured Logs

- [x] Add JSON line log format.
- [x] Add human dev log format.
- [x] Add log rotation.
- [x] Add log retention.
- [x] Add request ID in logs.
- [x] Add operation ID in logs.
- [x] Add redaction helper.
- [x] Redact secret values.
- [x] Redact credentialed remote URLs.
- [x] Redact local paths in diagnostic mode when requested.
- [x] Add log tests for redaction.

### M2.7 Service Templates

- [x] Add macOS LaunchAgent template.
- [x] Add Linux systemd user service template.
- [x] Add Windows background process/service design note.
- [x] Add `devrelay agent install --dry-run`.
- [x] Add `devrelay agent install` for macOS dev mode.
- [x] Add `devrelay agent install` for Linux dev mode.
- [x] Add `devrelay agent uninstall`.
- [x] Add `devrelay agent status`.
- [x] Document manual Windows startup setup until packaged.
- [x] Add service template tests where practical.

### M2.8 Diagnostics

- [x] Add diagnostic bundle command.
- [x] Include version/capability.
- [x] Include redacted config.
- [x] Include recent structured logs.
- [x] Include state machine records placeholder.
- [x] Include Git command exit codes.
- [x] Include timing data.
- [x] Exclude source code by default.
- [x] Exclude snapshot objects by default.
- [x] Add `--include-sensitive-paths` explicit option.
- [x] Add diagnostic redaction tests.

### M2 Exit Gate

- [x] CLI can operate entirely through the agent.
- [x] Agent restart preserves project and snapshot state.
- [x] IPC access is scoped to the local user.
- [x] Event stream reconnect works.
- [x] Diagnostic bundle excludes source code by default.
- [x] Agent tests pass in CI.

## M3. Anchor Metadata And Single-Writer Lease

### M3.1 Anchor Mode

- [x] Add anchor mode config field.
- [x] Add `devrelay anchor init`.
- [x] Add `devrelay anchor status`.
- [x] Add anchor data directory layout.
- [x] Add anchor metadata DB path.
- [x] Add anchor snapshot repo root.
- [x] Add anchor CAS root.
- [x] Add anchor startup path.
- [x] Add agent role detection.
- [x] Add role-specific health output.

### M3.2 Metadata Schema

- [x] Add `devices` table.
- [x] Add `projects` table compatibility with local registry.
- [x] Add `workspaces` table with device ID.
- [x] Add `sessions` table.
- [x] Add `snapshots` table with sequence number.
- [x] Add `leases` table.
- [x] Add `handoffs` table.
- [x] Add `task_runs` placeholder table.
- [x] Add foreign keys where appropriate.
- [x] Add indexes for lease lookup.
- [x] Add indexes for latest snapshot lookup.
- [x] Add indexes for handoff lookup.
- [x] Add schema migration tests.
- [x] Add WAL mode test.

### M3.3 Device Identity Placeholder

- [x] Generate local device ID.
- [x] Store device display name.
- [x] Store platform key.
- [x] Store architecture.
- [x] Store capabilities JSON.
- [x] Store paired timestamp placeholder.
- [x] Store last seen timestamp.
- [x] Add `devrelay devices list`.
- [x] Add `devrelay device show`.
- [x] Add device identity tests.

### M3.4 Session Model

- [x] Add session ID generation.
- [x] Add default session creation on project add.
- [x] Link session to project.
- [x] Store session name.
- [x] Store parent session ID.
- [x] Store archived timestamp.
- [x] Add `devrelay sessions list`.
- [x] Add `devrelay session show`.
- [x] Add `devrelay session fork`.
- [x] Add `devrelay session archive`.
- [x] Add session tests.

### M3.5 Lease State Machine

- [x] Define lease states as enum.
- [x] Add active state.
- [x] Add handoff pending state.
- [x] Add committing state.
- [x] Add inactive state.
- [x] Add forked state.
- [x] Add archived state.
- [x] Add epoch field.
- [x] Add holder device ID field.
- [x] Add latest snapshot ID field.
- [x] Add handoff ID field.
- [x] Add state transition validation.
- [x] Add illegal transition tests.
- [x] Add epoch monotonicity tests.

### M3.6 Canonical Publish

- [x] Add publish transaction.
- [x] Persist snapshot metadata.
- [x] Verify session ID.
- [x] Verify lease epoch.
- [x] Verify holder device ID.
- [x] Verify lease state active.
- [x] Compare-and-swap latest snapshot ID.
- [x] Preserve stale snapshot without making it latest.
- [x] Return stale publish error/safe action.
- [x] Add stale epoch test.
- [x] Add wrong holder test.
- [x] Add inactive holder test.
- [x] Add concurrent publish test.

### M3.7 Handoff Protocol

- [x] Add handoff ID generation.
- [x] Add begin handoff transaction.
- [x] Store expected epoch.
- [x] Store source device ID.
- [x] Store target device ID.
- [x] Store source generation.
- [x] Store expiration.
- [x] Add target prepare state.
- [x] Add target verified state.
- [x] Add source ready state.
- [x] Add commit state.
- [x] Add abort state.
- [x] Increment lease epoch on commit.
- [x] Change holder on commit.
- [x] Reject commit if source generation changed.
- [x] Reject commit if handoff expired.
- [x] Add handoff happy path test.
- [x] Add source-change abort test.
- [x] Add target-apply-failure test.
- [x] Add concurrent handoff test.

### M3.8 Inactive Edit Fork

- [x] Detect publish attempt from inactive workspace.
- [x] Create fork session.
- [x] Preserve inactive changes as snapshot.
- [x] Pin fork snapshot by default.
- [x] Emit `session.diverged`.
- [x] Add CLI output for "separate work".
- [x] Add test that canonical latest is unchanged.
- [x] Add test that fork snapshot is recoverable.

### M3.9 Crash Recovery

- [x] Add handoff journal table or records.
- [x] Record begin handoff.
- [x] Record target prepare.
- [x] Record target apply.
- [x] Record target verified.
- [x] Record lease committed.
- [x] Resume incomplete handoff safely.
- [x] Abort expired incomplete handoff safely.
- [x] Add crash-before-commit test.
- [x] Add crash-after-commit test.

### M3 Exit Gate

- [x] Stale lease publish cannot advance canonical latest.
- [x] Concurrent handoff attempts resolve deterministically.
- [x] Target dirty protection happens before lease transfer.
- [x] Crash between apply and lease commit is recoverable.
- [x] Inactive edit becomes fork, not canonical change.

## M4. LAN Pairing And Secure Control Plane

### M4.1 Fabric Identity

- [x] Select crypto crates.
- [x] Generate fabric root key.
- [x] Store fabric root key securely in dev mode.
- [x] Generate device signing key.
- [x] Generate network certificate key.
- [x] Store device public identity.
- [x] Store root public identity.
- [x] Add recovery export placeholder.
- [x] Add identity rotation design note.
- [x] Add identity serialization tests.

### M4.2 Pairing Protocol

- [x] Define pairing session schema.
- [x] Generate ephemeral pairing key.
- [x] Start pairing session on new device.
- [x] Discover anchor or accept manual address.
- [x] Compute handshake transcript.
- [x] Derive short authentication string.
- [x] Show code on both devices.
- [x] Require user confirmation.
- [x] Issue device certificate.
- [x] Persist paired device.
- [x] Expire old pairing sessions.
- [x] Add pairing abort.
- [x] Add pairing replay test.
- [x] Add mismatched-code test.

### M4.3 mDNS Discovery

- [x] Choose mDNS crate.
- [x] Advertise `_devrelay-anchor._tcp.local`.
- [x] Advertise `_devrelay-peer._tcp.local`.
- [x] Include `protocol=1`.
- [x] Include truncated fabric hint.
- [x] Include public device ID.
- [x] Include port.
- [x] Exclude project names.
- [x] Exclude repository paths.
- [x] Exclude usernames.
- [x] Add discovery browser.
- [x] Add manual address fallback.
- [x] Add mDNS disable config.
- [x] Add privacy test for TXT records.

### M4.4 mTLS Transport

- [x] Add rustls server config.
- [x] Add rustls client config.
- [x] Require TLS for control channel.
- [x] Validate device certificate.
- [x] Pin fabric root.
- [x] Check revocation denylist.
- [x] Add protocol version negotiation.
- [x] Add request timestamp.
- [x] Add replay nonce.
- [x] Add max clock skew policy.
- [x] Add connection timeout.
- [x] Add request timeout.
- [x] Add expired cert test.
- [x] Add revoked cert test.
- [x] Add wrong fabric test.

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

- [x] Add device revoke command.
- [x] Add revocation record.
- [x] Reject revoked device connection.
- [x] Reject revoked publish.
- [x] Reject revoked lease operation.
- [x] Emit audit event.
- [x] Add key rotation option placeholder.
- [x] Add revoke tests.

### M4.7 Audit Log

- [x] Record pair event.
- [x] Record revoke event.
- [x] Record snapshot publish event.
- [x] Record snapshot apply event.
- [x] Record lease transfer event.
- [x] Record command approval event.
- [x] Record security block event.
- [x] Add audit query CLI.
- [x] Add audit export with redaction.
- [x] Add audit tests.

### M4 Exit Gate

- [x] Pairing requires matching displayed code.
- [x] Revoked device cannot connect.
- [x] mDNS TXT records do not leak sensitive data.
- [x] Transport tests cover expired, wrong-fabric, and revoked certificates.
- [x] Control API rejects unauthenticated requests.

## M5. Data Plane: Git Object Transfer And Sidecar CAS

### M5.1 Git Object Data Plane

- [x] Choose first implementation strategy.
- [x] Define allowed ref namespace.
- [x] Serve project snapshot bare repo.
- [x] Restrict fetch to `refs/devrelay/*`.
- [x] Restrict push to permitted snapshot refs.
- [x] Enforce project authorization.
- [x] Enforce object size limits.
- [x] Enforce repository quota.
- [x] Add object availability check.
- [x] Add object corruption detection.
- [x] Add data-plane integration tests.

### M5.2 Anchor Snapshot Repos

- [x] Create anchor project repo on project registration.
- [x] Store snapshot refs in anchor repo.
- [x] Fetch source snapshot into anchor repo.
- [x] Serve target fetch from anchor repo.
- [x] Add orphan snapshot scan.
- [x] Add anchor repo maintenance command.
- [x] Add anchor repo GC guard.
- [x] Add anchor repo tests.

### M5.3 Route Selection

- [x] Measure source online status.
- [x] Measure anchor availability.
- [x] Add direct peer route.
- [x] Add anchor cache route.
- [x] Add source-required route.
- [x] Add route decision explanation.
- [x] Add route fallback on failure.
- [x] Add route metrics.
- [x] Add direct route tests.
- [x] Add anchor fallback tests.

### M5.4 CAS Core

- [x] Define chunk hash format.
- [x] Define CAS manifest schema.
- [x] Add chunk missing query.
- [x] Add chunk upload endpoint.
- [x] Add chunk download endpoint.
- [x] Verify chunk hash on upload.
- [x] Verify chunk hash on download.
- [x] Store chunk atomically.
- [x] Deduplicate chunks.
- [x] Add manifest create endpoint.
- [x] Add manifest fetch endpoint.
- [x] Add CAS reachability root concept.
- [x] Add CAS tests.

### M5.5 Large Sidecars

- [x] Detect untracked file over threshold.
- [x] Store large accepted file in CAS.
- [x] Add content-defined chunking placeholder or fixed chunk first pass.
- [x] Bound upload buffer memory.
- [x] Bound download buffer memory.
- [x] Add sidecar root hash.
- [x] Add sidecar file mode.
- [x] Add sidecar logical path.
- [x] Add sidecar classification.
- [x] Add sidecar manifest to snapshot metadata.
- [x] Add large file round-trip test.
- [x] Add corrupt chunk test.
- [x] Add missing chunk test.

### M5.6 Sidecar Materialization

- [x] Fetch required CAS manifest before apply.
- [x] Query missing chunks.
- [x] Download missing chunks.
- [x] Verify chunks.
- [x] Materialize sidecar file atomically.
- [x] Restore file mode where supported.
- [x] Prevent path traversal.
- [x] Prevent symlink escape.
- [x] Verify materialized root hash.
- [x] Add materialization tests.

### M5.7 Partial Upload Safety

- [x] Mark snapshot data upload as pending.
- [x] Upload Git objects before metadata publish.
- [x] Upload CAS chunks before metadata publish.
- [x] Verify anchor has required data.
- [x] Publish metadata only after data availability.
- [x] Ensure partial upload does not update latest.
- [x] Add network cut fault test.
- [x] Add upload retry test.
- [x] Add orphan cleanup test.

### M5 Exit Gate

- [x] Large sidecar transfer uses bounded memory.
- [x] Missing chunk blocks apply before lease transfer.
- [x] Anchor can serve snapshot after source offline.
- [x] Partial upload never changes canonical latest.
- [x] Data plane enforces project authorization.

## M6. Background Protection

### M6.1 Filesystem Watcher

- [x] Define watcher trait.
- [x] Implement macOS watcher.
- [ ] Implement Linux watcher.
- [ ] Implement Windows watcher.
- [x] Add polling fallback for unsupported platforms.
- [x] Treat events as hints only.
- [x] Increment source generation on relevant events.
- [x] Coalesce path sets.
- [x] Drop events outside registered workspaces.
- [x] Add watcher lifecycle management.
- [x] Add watcher tests with synthetic events.

### M6.2 Adaptive Debounce

- [x] Add first-event quiet timer.
- [x] Add minimum checkpoint interval.
- [x] Add max dirty interval.
- [x] Add publish quiet timer.
- [x] Add max publish interval.
- [x] Add immediate flush on explicit checkpoint.
- [x] Add immediate flush on handoff.
- [x] Add immediate flush on sleep/lock signal where available.
- [x] Add debounce tests.
- [x] Add coalescing tests.

### M6.3 Background Checkpoint

- [x] Track dirty workspace state.
- [x] Trigger Git status after quiet window.
- [x] Skip checkpoint if semantic state unchanged.
- [x] Create local snapshot.
- [x] Publish to anchor if available.
- [x] Emit protection status event.
- [x] Avoid notifications for normal success.
- [x] Surface repeated failures.
- [x] Add background checkpoint tests.

### M6.4 Git Performance Doctor

- [x] Detect Git version.
- [x] Detect FSMonitor support.
- [x] Detect existing FSMonitor config.
- [x] Detect untracked cache support.
- [x] Detect existing untracked cache config.
- [x] Add safe recommendation output.
- [x] Add `doctor --fix-safe` for approved config only.
- [x] Avoid overwriting user-managed config.
- [x] Add doctor tests.

### M6.5 Resource Policy

- [x] Define adaptive profile.
- [x] Define instant profile.
- [x] Define eco profile.
- [x] Define custom profile.
- [x] Add CPU slot limit.
- [x] Add hashing concurrency limit.
- [x] Add network bandwidth cap.
- [x] Add battery mode behavior.
- [x] Add low-power mode behavior.
- [x] Add foreground load detection.
- [x] Add resource policy persistence.
- [x] Add resource policy tests.

### M6.6 Retention And Quota

- [x] Define hot snapshot retention.
- [x] Define hourly thinning.
- [x] Define daily thinning.
- [x] Protect latest canonical snapshot.
- [x] Protect pinned snapshots.
- [x] Protect handoff snapshots for configured duration.
- [x] Add device cache quota.
- [x] Add anchor project quota.
- [x] Add free disk warning threshold.
- [x] Add free disk hard stop threshold.
- [x] Add pruning planner.
- [x] Add pruning executor.
- [x] Add retention tests.
- [x] Add quota tests.

### M6.7 Crash Journal

- [x] Add journal record type.
- [x] Record snapshot creation start.
- [x] Record snapshot creation complete.
- [x] Record publish start.
- [x] Record publish complete.
- [x] Record target apply start.
- [x] Record target backup complete.
- [x] Record base applied.
- [x] Record work applied.
- [x] Record index applied.
- [x] Record verified.
- [x] Record lease committed.
- [x] Add journal replay.
- [x] Add journal cleanup.
- [x] Add fault injection tests.

### M6 Exit Gate

- [ ] Idle agent CPU/RSS meets target on test repos.
- [x] Many file events coalesce into bounded work.
- [x] Disk pressure prunes only unpinned/evictable data.
- [x] Background failures surface as protection status.
- [x] Background watcher is not used as source of truth.

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

- [x] Define platform key format.
- [x] Detect Linux GNU x86_64.
- [x] Detect Linux aarch64.
- [x] Detect Darwin arm64.
- [x] Detect Darwin x86_64.
- [x] Detect Windows native x86_64.
- [x] Detect WSL2 Linux.
- [x] Detect WSL distro.
- [x] Detect WSL version.
- [x] Store platform capabilities.
- [x] Add platform detection tests.

### M11.2 Path Portability Doctor

- [x] Walk Git-tracked paths.
- [x] Walk accepted untracked paths.
- [x] Detect case-fold collisions.
- [x] Detect Unicode normalization collisions.
- [x] Detect Windows reserved names.
- [x] Detect trailing dot.
- [x] Detect trailing space.
- [x] Detect invalid Windows characters.
- [x] Detect path length budget violations.
- [x] Detect symlink capability mismatch.
- [x] Report path conflict codes.
- [x] Add target exclusion safe action.
- [x] Add rename guidance safe action.
- [x] Add portability tests.

### M11.3 Line Endings

- [x] Read `.gitattributes`.
- [x] Detect missing policy.
- [x] Detect conflicting `core.autocrlf`.
- [x] Verify by Git semantic hash.
- [x] Add warning for risky target config.
- [x] Add line-ending tests.

### M11.4 Executable Bit

- [x] Preserve Git tree mode.
- [x] Verify executable bit on POSIX.
- [x] Preserve index mode on Windows.
- [x] Add executable-bit tests.

### M11.5 Symlink Policy

- [x] Detect symlink entries.
- [x] Preserve symlink target string.
- [x] Refuse to follow symlink outside workspace.
- [x] Block materialization if target lacks symlink support.
- [x] Add symlink capability doctor.
- [x] Add symlink tests.

### M11.6 Windows Reparse Points

- [x] Detect reparse points in workspace.
- [x] Prevent reparse point traversal during scan.
- [x] Prevent reparse point traversal during materialization.
- [x] Add Windows-specific tests where possible.

### M11.7 WSL/Native Separation

- [x] Represent Windows native as separate device.
- [x] Represent WSL distro as separate device.
- [x] Prevent shared tree mutation warning.
- [x] Add workspace mapping guidance.
- [x] Add WSL filesystem doctor.
- [x] Add WSL tests where possible.

### M11 Exit Gate

- [x] Unsafe paths block before target apply.
- [x] Windows native and WSL are separate devices.
- [ ] Symlink/reparse traversal tests pass.
- [x] Line-ending verification uses Git semantics.

## M12. Advanced Git States

### M12.1 Merge/Cherry-Pick/Revert Conflicts

- [x] Detect merge conflict state.
- [x] Detect cherry-pick conflict state.
- [x] Detect revert conflict state.
- [x] Capture index stage 1 entries.
- [x] Capture index stage 2 entries.
- [x] Capture index stage 3 entries.
- [x] Capture operation metadata.
- [x] Store operation capsule.
- [x] Apply unmerged index to target.
- [x] Restore conflict markers.
- [x] Verify staged entries.
- [x] Add conflict round-trip tests.

### M12.2 Submodules

- [x] Detect submodule config.
- [x] Detect clean submodule state.
- [x] Restore clean submodule recorded commit.
- [x] Detect dirty submodule state.
- [x] Create child project/session for dirty submodule.
- [x] Store parent-child snapshot topology.
- [x] Add recursion depth limit.
- [x] Add cycle detection.
- [x] Add submodule tests.

### M12.3 Git LFS

- [x] Detect LFS pointer files.
- [x] Detect required local LFS objects.
- [x] Check upstream availability.
- [x] Store local-only LFS object in CAS.
- [x] Verify LFS object before apply.
- [x] Block apply if object missing.
- [x] Add LFS tests with fake objects.

### M12.4 Sparse Checkout And Partial Clone

- [x] Detect sparse checkout.
- [x] Capture sparse definition as workspace preference.
- [x] Distinguish logical snapshot from sparse view.
- [x] Fetch missing blobs on demand.
- [x] Avoid overwriting target sparse policy.
- [x] Add sparse checkout tests.
- [x] Add partial clone tests where practical.

### M12.5 Interactive Rebase And Sequencer

- [x] Detect interactive rebase.
- [x] Detect sequencer state.
- [x] Capture original HEAD.
- [x] Capture onto commit.
- [x] Capture todo list.
- [x] Capture current step.
- [x] Define target Git version compatibility.
- [ ] Reconstruct operation on target.
- [x] Add safe block fallback.
- [x] Add operation capsule tests.
- [x] Keep feature disabled until tests are exhaustive.

### M12 Exit Gate

- [x] Conflict round-trip preserves staged entries.
- [x] Unsupported operations block with recovery options.
- [x] Rebase support remains gated until exhaustive tests pass.
- [x] Advanced states never silently degrade into normal snapshots.

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

- [x] Add fuzz target for porcelain parser.
- [x] Add fuzz target for manifest parser.
- [x] Add fuzz target for path canonicalization.
- [x] Add fuzz target for CAS manifest parser.
- [x] Add fuzz target for network API payloads.
- [x] Add fuzz corpus seeds.
- [x] Add fuzz run documentation.
- [x] Add CI/nightly fuzz plan.

### M13.3 Secret Scanning

- [x] Expand sensitive filename rules.
- [x] Add private key header scanner.
- [x] Add token pattern scanner.
- [x] Add high-entropy heuristic.
- [x] Add user-configured scanner hook.
- [x] Add raw secret redaction tests.
- [ ] Add false-positive override design.
- [ ] Add encrypted one-time sidecar design.

### M13.4 Update And Migration

- [ ] Define release channel model.
- [ ] Define signed release strategy.
- [ ] Define binary provenance strategy.
- [ ] Define metadata schema migration support window.
- [ ] Define snapshot schema migration support window.
- [ ] Add rollback expectations.
- [x] Add migration tests from old fixtures.

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

- [x] Include version/capability.
- [x] Include redacted config.
- [x] Include recent structured logs.
- [x] Include state machine records.
- [x] Include Git command exit codes.
- [x] Include timing metrics.
- [x] Exclude source code by default.
- [x] Exclude snapshot objects by default.
- [x] Add explicit sensitive export option.
- [x] Add diagnostic tests.

### M13.8 Fault Injection

- [x] Inject crash during snapshot object write.
- [x] Inject crash during ref update.
- [x] Inject network cut during publish.
- [x] Inject crash during metadata transaction.
- [x] Inject crash during target fetch.
- [x] Inject crash during base apply.
- [x] Inject crash during work apply.
- [x] Inject crash during index apply.
- [x] Inject crash during verification.
- [x] Inject crash during lease commit.
- [x] Inject disk-full during CAS upload.
- [x] Inject disk-full during target apply.
- [x] Add zero-data-loss assertions.

### M13 Exit Gate

- [ ] Independent security review has no unresolved critical findings.
- [x] Revocation blocks new access and lease operations.
- [x] Diagnostic export excludes source/snapshot by default.
- [x] Fault injection produces zero data loss in supported states.
- [x] Secret hard-block regression count is zero.

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
