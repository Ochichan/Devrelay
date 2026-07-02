# Current State

Last updated: 2026-07-02

This document is the short status source for contributors. The North Star spec
describes the target product; this file describes what the repository currently
supports, where the documentation gates are still open, and what should happen
next.

## Summary

DevRelay has enough core depth to start a product UX vertical slice. Continuing
to add advanced core features before dogfooding the handoff experience now has
diminishing value. The next high-value defects are likely to be product defects:
unclear writer ownership, uncertain handoff progress, missing target readiness,
or anxiety around dirty target preservation.

## Milestone Status

| Milestone | Status | Notes |
| --- | --- | --- |
| M0 Git state round trip | Complete | Local snapshot create/apply/verify is implemented and covered by round-trip fixtures. |
| M1 Local CLI, SQLite, recovery | Complete | Project registry, snapshot store, recovery, dirty policies, and local continue flow are implemented. |
| M2 Agent, IPC, RPC, events | Nearly complete | Agent, JSON-RPC, local lease and handoff state RPC, event stream with handoff state events, diagnostics, local metrics export, and macOS/Linux local IPC exist. Windows named pipe and pipe ACL remain open. |
| M3 Anchor and single-writer lease | Complete | Canonical publish, stale publish, handoff, inactive edit fork, and crash recovery are implemented. |
| M4 Pairing, mTLS, revocation | Complete | Identity, pairing, discovery, mTLS transport, revocation, audit, and the remote Control API are implemented. The agent serves the M4.5 remote JSON-RPC boundary over mTLS with per-request certificate validation, TLS key binding, replay/skew checks, and security-blocked audit records, covered by integration tests. Real two-device manual boundary evidence still runs through the manual verification runbook. |
| M5 Git object and CAS data plane | Complete | Per-project bare repo strategy, route selection, CAS, sidecars, materialization, and partial upload safety are implemented. |
| M6 Background protection | Nearly complete | Debounce, checkpoint, resource policy, retention, quota, crash journal, and native macOS/Linux filesystem watchers exist. Representative macOS resource evidence meets the numeric idle/burst budgets (see docs/resource-benchmark.md); Windows watcher coverage plus Linux/battery/anchor resource scenarios remain open. |
| M7 Desktop UX | Started | A Tauri shell exists with tray status, refresh, continue targets, run shortcuts, background profile toggle, open/quit, reduced-motion handling, generated app icon placeholder, prototype-aligned visual polish, agent-backed bootstrap, event subscription status/gap recovery, lease-backed writer state, snapshot-backed checkpoint age, handoff state visibility, target readiness, keyboard-safe handoff review, screen-reader action labels, dirty-target-safe copy, source-side prepare/abort actions, target-side apply/verify/commit, project status, checkpoint, diagnostics, settings, and overflow-tested screens. The first-slice macOS/Linux policy, agent contract, activity failure payload, and dogfood scripts are documented. Real remote run dispatch and real-device cross-device dogfood evidence remain open. |
| M8 Editor context | Started | VS Code extension package, TypeScript build, linting, activation events, local agent IPC client, writer/handoff/protection-delayed status bar state, command palette actions, workspace/editor/tab/breakpoint/terminal context capture, size limits, `editor.context.update`/`editor.context.latest`/`editor.restore.ack` agent RPCs, and package/client/capture/status/restore/agent tests exist. Editor context capture can be disabled through `devrelay.captureEditorContext`, and Continue Elsewhere treats context capture as best-effort so guarded handoff start can proceed with a fallback source generation. Unsaved dirty buffer capture/restore exists as a local-only VS Code SecretStorage path with default-off settings and untitled-buffer opt-in. The extension now reports edit/save/active-editor events through `editor.event.record`; meaningful edits increment agent source generation and abort pending source handoffs. Context restore opens saved folders/files, active selections, split columns, breakpoints, and local unsaved buffers where VS Code APIs allow. Command palette entries cover continue here, continue elsewhere, checkpoint, run history, recovery timeline, explain state, and dashboard; task execution is still limited to recent run display because no start-run RPC exists. Real extension-host validation remains open. |
| M9 Environment hydration | Partial | Trust hashes, bootstrap fingerprint file content hashing, profile selection, Nix, Dev Container, native bootstrap adapter, secret provider materialization paths, the core hydration state machine with JSON persistence and `environment.progress` payloads, `environment.status`, `devrelay doctor environment`, `devrelay environment status`, and desktop Continue hydration visibility exist. Environment failure integrity is covered by the `environment_failure_leaves_code_intact` safety suite; representative shell-ready SLO evidence remains open. |
| M10 Compute fabric | Partial | Task definitions parse and validate profile references, commands, platform constraints, resource hints, and sandbox settings. Task-specific command definition hashes, pinned immutable task execution snapshots, task run metadata recording/listing, scheduler constraint filtering, explainable scheduler target selection, isolated runner workspace preparation, host task execution, redacted task log buffering/spooling, artifact capture/retrieval, result cache keying/lookup, Nix delegation planning, and code-changing task summarization exist. Remote runner dispatch remains open. |
| M11 Cross-platform hardening | Mostly complete | Platform identity, path doctor, line endings, executable bit, symlink/reparse, and WSL separation are implemented. |
| M12 Advanced Git states | Mostly complete | Conflicts, submodules, LFS, sparse/partial clone are implemented. Interactive rebase reconstruction remains gated. |
| M13 Security and operations | Partial | Threat model, fuzzing, diagnostics, fault injection, secret scanning pieces, false-positive override design, encrypted one-time sidecar design, release/update policy, backup anchor data set, and opaque anchor research decision exist. Backup anchor replication/restore and independent review remain open. |
| M14 Beta product loop | Started | Local metrics export now derives redacted, local-only aggregate reports from audit events, snapshots, handoff journals, task runs, and hydration state. The user guide covers quick start, pairing, project registration, handoff, recovery, dirty/inactive states, security, backup, and troubleshooting. Dev/beta channels, WSL agent instructions, upgrade, uninstall paths, and dogfood scenario selections are documented. Signed installers, onboarding, and real-device dogfood evidence remain open. |
| M15 Release candidate | Not started | Depends on product gates, resource evidence, UX evidence, and signed artifacts. |

## Documentation Corrections Made

The following previous document states were misleading:

- README still described the repository as Phase 0 only.
- Roadmap baseline said registry, agent, lease, network, and data-plane work did
  not exist.
- API surface only documented an M0 crate-root contract even though M1-M5
  structures are now exposed through the crate root.
- M4 exit gate said the Control API rejected unauthenticated requests while
  M4.5 itself was completely unchecked.
- M2 exit gate implied all local IPC access was scoped even though Windows pipe
  ACL work remains open.

## Product Direction

The next implementation path is:

1. Dogfood macOS/Linux first. Windows UI waits until named pipe transport and
   per-user pipe ACL are credible.
2. Broaden resource benchmark coverage before promising invisible protection.
3. Run the manual verification runbook, starting with the desktop runtime
   sub-runbook for the current Tauri shell.
4. Dogfood clean target handoff on real macOS/Linux devices.
5. Dogfood dirty target and inactive edit preservation.
6. Expand to Windows/WSL after Windows IPC and startup packaging are credible.

The non-negotiable safety rules are now integration suites with stable names;
see the evidence mapping in `docs/data-loss-safety.md`.

## Current Blockers Before Broad UI Claims

- Windows named pipe transport and per-user pipe ACL.
- Resource benchmark evidence on Linux plus battery, anchor, and
  watcher-driven scenarios.
- Manual verification evidence for desktop runtime, real-device dogfood,
  packaging, rollback, resource, UX, and security gates, including the remote
  Control API manual boundary checks on real devices.
