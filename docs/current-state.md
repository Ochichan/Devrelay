# Current State

Last updated: 2026-06-24

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
| M2 Agent, IPC, RPC, events | Nearly complete | Agent, JSON-RPC, local lease and handoff state RPC, event stream with handoff state events, diagnostics, and macOS/Linux local IPC exist. Windows named pipe and pipe ACL remain open. |
| M3 Anchor and single-writer lease | Complete | Canonical publish, stale publish, handoff, inactive edit fork, and crash recovery are implemented. |
| M4 Pairing, mTLS, revocation | Mostly complete | Identity, pairing, discovery, mTLS transport, revocation, and audit are present. M4.5 Control API remains unimplemented, so the M4 exit gate is open. |
| M5 Git object and CAS data plane | Complete | Per-project bare repo strategy, route selection, CAS, sidecars, materialization, and partial upload safety are implemented. |
| M6 Background protection | Nearly complete | Debounce, checkpoint, resource policy, retention, quota, and crash journal exist. Initial macOS resource smoke evidence exists; Linux/Windows watcher coverage and representative resource evidence remain open. |
| M7 Desktop UX | Started | A Tauri shell exists with tray status, refresh, continue targets, run shortcuts, background profile toggle, open/quit, reduced-motion handling, generated app icon placeholder, prototype-aligned visual polish, agent-backed bootstrap, event subscription status/gap recovery, lease-backed writer state, snapshot-backed checkpoint age, handoff state visibility, target readiness, keyboard-safe handoff review, screen-reader action labels, dirty-target-safe copy, source-side prepare/abort actions, target-side apply/verify/commit, project status, checkpoint, diagnostics, settings, and overflow-tested screens. The first-slice macOS/Linux policy, agent contract, activity failure payload, and dogfood scripts are documented. Real remote run dispatch and real-device cross-device dogfood evidence remain open. |
| M8 Editor context | Not started | Keep out of the first UI slice. |
| M9 Environment hydration | Partial | Trust hashes, profile selection, Nix, and Dev Container paths exist. Native bootstrap, secrets, hydration state, and doctor remain open. |
| M10 Compute fabric | Not started | Keep out of the first UI slice. |
| M11 Cross-platform hardening | Mostly complete | Platform identity, path doctor, line endings, executable bit, symlink/reparse, and WSL separation are implemented. |
| M12 Advanced Git states | Mostly complete | Conflicts, submodules, LFS, sparse/partial clone are implemented. Interactive rebase reconstruction remains gated. |
| M13 Security and operations | Partial | Fuzzing, diagnostics, fault injection, secret scanning pieces exist. Threat model and release operations remain open. |
| M14 Beta product loop | Not started | Depends on real-device dogfood. |
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
3. Convert non-negotiable safety rules into integration suites with stable
   names.
4. Run the manual desktop runtime checklist against the current Tauri shell.
5. Dogfood clean target handoff on real macOS/Linux devices.
6. Dogfood dirty target and inactive edit preservation.
7. Expand to Windows/WSL after Windows IPC and startup packaging are credible.

## Current Blockers Before Broad UI Claims

- Windows named pipe transport and per-user pipe ACL.
- Representative resource benchmark results beyond the initial macOS smoke run.
- Integration-level safety suites mapped to the non-negotiable checklist.
- Manual runtime evidence for the desktop shell and DMG.
- A clear Control API decision: implement M4.5 over TLS or rename the boundary
  if JSON-RPC over mTLS is the intended remote API.
