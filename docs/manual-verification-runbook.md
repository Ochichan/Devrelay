# Manual Verification Runbook

Last updated: 2026-06-24

This is the top-level manual runbook for checks that cannot be trusted from
unit tests alone: real devices, packaging, OS permissions, accessibility,
security posture, rollback, resource behavior, and user-comprehension gates.

Do not mark a North Star checklist item complete from this document unless the
run produced evidence. A planned scenario is not evidence.

## Evidence Ledger

Create one evidence record per run. Store local raw output under `target/` or
`output/` unless the evidence is accepted for a release gate.

Record:

- [ ] Run id.
- [ ] Date, local timezone, and operator.
- [ ] Git commit SHA.
- [ ] Branch and dirty-tree status before the run.
- [ ] Build channel: dev, nightly, beta, or release-candidate.
- [ ] Artifact paths and SHA-256 checksums when artifacts are involved.
- [ ] Device role, OS version, architecture, and filesystem.
- [ ] Agent launch mode: foreground, service, package, or app-managed.
- [ ] `DEVRELAY_HOME` path for every device.
- [ ] Network shape: same LAN, manual address, anchor, peer-only, or offline.
- [ ] Test project fixture description.
- [ ] Expected result.
- [ ] Actual result.
- [ ] Pass/fail/block decision.
- [ ] Diagnostic bundle path if exported.
- [ ] Screenshots or terminal excerpts for failures.
- [ ] Follow-up checklist item, issue, or commit.

Failure evidence must state whether user work was preserved.

## Execution Order

Use this order for a full dogfood or release-candidate pass:

1. Local automated baseline.
2. Desktop runtime manual checklist.
3. Single-device safety smoke.
4. Onboarding and setup.
5. Remote Control API boundary, when the server exists.
6. Pairing, discovery, and revocation.
7. Real-device macOS/Linux dogfood.
8. Cross-platform Windows/WSL pass, after Windows IPC/startup is credible.
9. Editor context restore.
10. Environment hydration.
11. Compute fabric.
12. Advanced Git states.
13. Backup anchor.
14. Resource and performance.
15. UX and accessibility.
16. Security review.
17. Packaging, update, rollback, and release-candidate gates.

If a prerequisite is not implemented, mark the run `blocked` and link the open
North Star checklist item. Do not substitute mock evidence for real-device
evidence.

## Local Baseline

Run before any manual product claim:

```bash
git status --short
git rev-parse HEAD
RUSTC_WRAPPER= cargo test -p devrelay-core remote_rpc --lib
RUSTC_WRAPPER= cargo test -p devrelay-agent -p devrelay-desktop
npm run check:ui --prefix apps/desktop
git diff --check
```

Pass:

- [ ] The working tree has no unrelated dirty changes.
- [ ] Focused tests for the touched area pass.
- [ ] Agent and desktop tests pass.
- [ ] UI check passes.
- [ ] `git diff --check` reports no whitespace errors.
- [ ] Any hung or skipped command is recorded with process details.

## Desktop Runtime

Primary runbook:

- [manual-runtime-checklist.md](manual-runtime-checklist.md)

Run it for:

- [ ] Browser fallback mode.
- [ ] Tauri app without agent.
- [ ] Tauri app with foreground agent.
- [ ] Disposable project registration.
- [ ] Status, checkpoint, diagnostics, and settings flows.
- [ ] Event stream reconnect.
- [ ] DMG mount and copied-app launch.
- [ ] Overflow and keyboard smoke.
- [ ] Idle CPU/RSS smoke.

This is the gate for local desktop dogfood only. It does not prove cross-device
handoff.

## Single-Device Safety Smoke

Use a disposable Git repository.

Manual checks:

- [ ] Dirty tracked files remain dirty after status, checkpoint, diagnostics,
  and open-folder actions.
- [ ] Safe untracked files are visible in status/checkpoint evidence.
- [ ] Secret-like files are excluded from snapshots and diagnostics.
- [ ] Missing project path errors are visible and do not create success toasts.
- [ ] Recovery timeline can list snapshots without exposing raw source files.
- [ ] Diagnostic bundles are redacted by default.
- [ ] Audit events contain stable method/outcome information.

Evidence:

- `git status --short` before and after each operation.
- CLI JSON output for status/checkpoint/recover/diagnostics.
- Screenshot of corresponding UI state when UI is involved.

## Onboarding And Setup

Prerequisite: onboarding screens are implemented in the product surface being
tested.

Manual checks:

- [ ] First-device flow creates or selects a fabric without command-line setup.
- [ ] Anchor selection flow explains always-on expectations and storage impact.
- [ ] Peer-only selection flow explains limitations before the user proceeds.
- [ ] Existing-anchor connection flow accepts discovered and manual addresses.
- [ ] Short authentication string confirmation is visible, legible, and blocks
  mismatches.
- [ ] Project discovery flow finds likely workspaces without registering random
  folders.
- [ ] Environment detection flow explains Nix, Dev Container, native bootstrap,
  trusted script, manual profile, and missing-tool states.
- [ ] Command trust prompt shows command scope and hash-change reason.
- [ ] Recovery key export prompt makes storage responsibility explicit.
- [ ] User can complete onboarding with keyboard only.

Evidence:

- Screen recordings or screenshots for each flow.
- Final config redacted for paths/secrets.
- Notes for any step where the user needed docs or terminal commands.

## Remote Control API

Prerequisite: met. The agent serves the remote JSON-RPC boundary over mTLS
via `devrelay-agent --remote-listen`; see `docs/remote-rpc-api.md`.

Manual checks:

- [ ] Unauthenticated TCP/TLS connection is rejected before method dispatch.
- [ ] Certificate from the wrong fabric is rejected.
- [ ] Revoked device certificate is rejected.
- [ ] Expired device certificate is rejected.
- [ ] Replay nonce reuse is rejected.
- [ ] Clock-skewed request is rejected.
- [ ] Request without JSON-RPC id is rejected.
- [ ] Unknown method returns JSON-RPC `method not found`.
- [ ] DevRelay errors map to JSON-RPC error data with code, detail, and safe
  actions.
- [ ] `devices.list`, `projects.list`, `workspaces.list`, and
  `sessions.snapshots.list` return remote-safe fields only.
- [ ] Handoff methods enforce actor role before mutation.
- [ ] Recovery methods do not reveal unrelated local paths.

Evidence:

- Request/response transcript with request ids.
- Server logs proving auth/preflight happened before dispatch.
- `devrelay device revoke` evidence for the revoked-device case.

How to drive the boundary on real devices:

```bash
# owner device
devrelay-agent --foreground --remote-listen 0.0.0.0:0   # address lands in agent-remote.addr
devrelay remote credentials issue <pairing-id> --out peer-credentials.json

# paired device
devrelay remote credentials import peer-credentials.json
devrelay remote call devices.list --address <owner-host:port> --json
devrelay remote call settings.update --address <owner-host:port>   # expect method-not-found
```

Rejected requests appear as `security.blocked` audit events on the owner
agent (`devrelay audit list`), which is the server-side evidence trail.

Blocks:

- M4.5 code and API integration tests are complete. This manual boundary
  evidence remains a security-gate prerequisite for broad LAN trust claims
  and the release candidate.

## Pairing, Discovery, And Revocation

Run on two real devices.

Manual checks:

- [ ] New device pairing works without editing config by hand.
- [ ] Short authentication string appears on both devices and must match.
- [ ] Mismatched code aborts pairing.
- [ ] mDNS discovery finds the expected role without leaking project names,
  repository paths, or usernames.
- [ ] Manual/static discovery fallback works when mDNS is disabled or blocked.
- [ ] Revoked device disappears or is marked unusable in UI/CLI state.
- [ ] Revoked device cannot publish, receive handoff, or call remote Control
  API.

Evidence:

- Pairing CLI/UI screenshots.
- Discovery TXT record output.
- Revocation audit event.
- Failed post-revocation operation transcript.

## Real-Device Dogfood

Scenario source:

- [dogfood-scenarios.md](dogfood-scenarios.md)

Required first pass:

- [ ] macOS source to Linux target clean handoff.
- [ ] Linux source to macOS target clean handoff.
- [ ] Target dirty preservation.
- [ ] Inactive edit preservation.
- [ ] Anchor offline before handoff begins.
- [ ] Anchor offline after source checkpoint upload.
- [ ] Cold environment target.
- [ ] Defects recorded.
- [ ] Defects fed back into the checklist.

Pass:

- [ ] One-click handoff works between two real devices.
- [ ] Normal handoff requires no Git command.
- [ ] Project registry is consistent across devices.
- [ ] Workspace mapping is correct for each device.
- [ ] Single canonical writer lease is visible and moves only after verify.
- [ ] Explicit checkpoint is visible before handoff.
- [ ] Background checkpoint evidence is visible when enabled.
- [ ] Direct snapshot transfer works when source and target are online.
- [ ] Anchor snapshot transfer works when anchor is available.
- [ ] Target applies and verifies before writer ownership moves.
- [ ] Dirty target work is preserved or left unchanged.
- [ ] Inactive edits are preserved as separate recoverable work.
- [ ] Timeline recovery can open a snapshot as a new session.
- [ ] Activity details are understandable without raw Git object IDs.

Evidence:

- Device matrix.
- Per-step expected/actual notes.
- `git status --short` on source and target before/after.
- Audit log export.
- Diagnostic bundle from each device.

## Windows And WSL

Prerequisite: Windows named pipe transport, pipe ACL, watcher, and startup path
are implemented.

Manual checks:

- [ ] Windows named pipe accepts only the local user.
- [ ] Non-owner access to the pipe is rejected.
- [ ] Windows agent service starts after login/reboot.
- [ ] Windows filesystem watcher reports changes without duplicate storms.
- [ ] Windows native source to Linux target handoff works.
- [ ] WSL to macOS handoff works.
- [ ] WSL uses a separate Linux identity and `DEVRELAY_HOME`.
- [ ] Workspace on Windows mount from WSL triggers doctor warning.
- [ ] Path doctor reports reserved names, case collisions, Unicode issues, and
  long paths.

Evidence:

- Windows user/session ids.
- Pipe ACL output.
- Agent service status after reboot.
- Doctor JSON output.
- Handoff transcript.

## Cross-Platform Doctors And Diagnostics

Run on every supported OS before broad support claims.

Manual checks:

- [ ] `devrelay doctor git-performance --json` reports actionable safe actions.
- [ ] `devrelay doctor git-performance --fix-safe` mutates only documented
  unset safe Git config.
- [ ] `devrelay doctor paths --json` reports path risks on the checked
  platform.
- [ ] `devrelay doctor line-endings --json` reports risky autocrlf/policy
  combinations.
- [ ] `devrelay doctor wsl --json` distinguishes Windows-native, WSL distro,
  and mounted filesystem cases.
- [ ] `devrelay doctor environment --json` reports missing tools, trust
  changes, and required secret mappings.
- [ ] `devrelay diagnostics export --json` creates a bundle.
- [ ] Diagnostic bundle omits source files, snapshot Git objects, raw secrets,
  credentials, and unredacted local paths by default.
- [ ] Doctor failures give safe next actions.

Evidence:

- Doctor JSON output per OS.
- Diagnostic bundle manifest.
- Redaction spot-check notes.

## Editor Context

Run with VS Code installed.

Manual checks:

- [ ] Source captures workspace, active files, selections, terminals where
  supported, breakpoints, and dirty buffer opt-in state.
- [ ] Target opens the expected folder and active editor context.
- [ ] Missing or unsupported editor context falls back without blocking handoff.
- [ ] Restored context is acknowledged through `editor.restore.ack`.
- [ ] Handoff still works when editor context capture is disabled.

Evidence:

- Source context payload summary without sensitive file contents.
- Target screenshot after restore.
- Agent audit/event records for capture and restore acknowledgement.

## Environment Hydration

Run on representative projects for Nix, Dev Container, native bootstrap,
trusted script, manual profile, and secret materialization.

Manual checks:

- [ ] Environment failure leaves code state intact.
- [ ] Warm target enters dev shell within the SLO.
- [ ] Cold target reports hydration progress and actionable failure.
- [ ] Trust prompt blocks changed native/trusted commands until approved.
- [ ] Secret materialization creates only declared files/env vars.
- [ ] Missing required secret blocks execution with redacted diagnostics.
- [ ] Nix hydration works on a representative flake.
- [ ] Native bootstrap works on a representative project.

Evidence:

- `devrelay environment status --json`.
- Hydration state file excerpt.
- Before/after `git status --short`.
- Shell-ready timings.
- Redacted logs.

## Compute Fabric

Prerequisite: remote runner dispatch is implemented.

Manual checks:

- [ ] Remote task runner starts task from immutable execution snapshot.
- [ ] Scheduler selects an eligible target and explains rejected candidates.
- [ ] Artifact return works and rejects path traversal.
- [ ] Code-changing task runs in noncanonical runner workspace.
- [ ] Failed/timed-out task logs are redacted and retrievable.
- [ ] Cache hit does not reuse read/write or secret-sensitive outputs
  incorrectly.

Evidence:

- Task run JSON.
- Scheduler explanation.
- Artifact index.
- Source canonical workspace status before/after.

## Advanced Git States

Manual checks:

- [ ] Supported conflict state is rejected or preserved according to
  [supported-states.md](supported-states.md).
- [ ] Submodule dirty state round-trips with child snapshot topology intact.
- [ ] Git LFS local-only object is either transferred through sidecars or
  rejected before target mutation.
- [ ] Sparse checkout target preserves sparse policy.
- [ ] Partial clone fetches required blobs on demand or fails before mutation.
- [ ] Interactive rebase reconstruction remains disabled until exhaustive tests
  pass.
- [ ] Reconstruct operation on target is tested only after the feature is
  intentionally enabled.

Evidence:

- Fixture description.
- `git status --porcelain=v2 -z` before and after.
- Snapshot/recovery JSON.
- Explicit note when an advanced state is intentionally gated.

## Backup Anchor

Prerequisite: backup replication, signing, promotion, and restore commands are
implemented.

Manual checks:

- [ ] Metadata DB replication completes with integrity verification.
- [ ] Snapshot bare repo replication includes expected refs.
- [ ] CAS chunk replication includes every referenced chunk.
- [ ] Replicated state is signed and signature verification fails on tamper.
- [ ] Manual promotion command requires explicit operator confirmation.
- [ ] Promotion emits an audit event.
- [ ] Backup restore recreates a usable recovery timeline.
- [ ] Restore never prunes pinned/latest data.

Evidence:

- Replication manifest.
- Signature verification output.
- Promotion audit event.
- Restore command output and recovered workspace status.

## Resource And Performance

Primary plan:

- [resource-benchmark.md](resource-benchmark.md)

Manual checks:

- [ ] Representative resource benchmark results are recorded.
- [ ] Idle agent CPU/RSS meets target on 0, 10, and 50 registered projects.
- [ ] Background network cap is measured.
- [ ] Battery behavior is measured.
- [ ] Low-power behavior is measured.
- [ ] Quota/GC behavior is measured.
- [ ] Pinned/latest data protection is verified under quota pressure.
- [ ] Prefetched target code-ready p95 is under 5 seconds.
- [ ] Warm shell-ready p95 is under 15 seconds.

Evidence:

- Resource benchmark output.
- Platform/device metadata.
- p95 calculation.
- Accepted resource report.

## UX And Accessibility

Manual checks:

- [ ] New user pairs a device without docs.
- [ ] Handoff can be completed without Git commands.
- [ ] Target dirty resolution is understandable.
- [ ] Recovery timeline is understandable.
- [ ] Keyboard-only handoff works.
- [ ] Screen reader labels name critical actions.
- [ ] Reduced motion is respected.
- [ ] Actionable error comprehension is tested with a person other than the
  implementer when possible.
- [ ] Tauri tray/dashboard works from a packaged app.

Evidence:

- Participant notes or self-test notes.
- Screenshots.
- Accessibility tree or screen-reader notes.
- UX report.

## Security Review

Manual checks:

- [ ] mTLS is mandatory.
- [ ] Snapshot metadata signatures verify and fail on tamper.
- [ ] Command trust hash enforcement blocks changed commands.
- [ ] Revocation enforcement is visible across local and remote paths.
- [ ] Diagnostic redaction removes paths, secrets, tokens, and credentials.
- [ ] Path traversal defenses reject malicious snapshots/artifacts.
- [ ] Secret scanning defenses block default secret classes.
- [ ] Replay defenses reject reused nonces.
- [ ] Independent security review has no unresolved critical findings.

Evidence:

- Security report.
- Review findings and disposition.
- Redaction before/after examples.
- Negative test transcripts.

## Packaging, Update, Rollback, And Release Candidate

Policy source:

- [release-update-policy.md](release-update-policy.md)
- [install-update.md](install-update.md)
- [migration-rollback-policy.md](migration-rollback-policy.md)

Manual checks:

- [ ] macOS agent is packaged.
- [ ] macOS desktop app is packaged.
- [ ] Linux agent is packaged.
- [ ] Linux desktop app is packaged.
- [ ] Windows agent is packaged.
- [ ] Windows desktop app is packaged.
- [ ] Signed binaries are built.
- [ ] Installer artifacts are built.
- [ ] Artifact checksums are published.
- [ ] macOS codesign/notarization verification passes.
- [ ] Linux signature verification passes.
- [ ] Windows Authenticode verification passes before Windows support claims.
- [ ] Docs site or docs bundle is built.
- [ ] Known issues list is produced.
- [ ] Rollback instructions are produced.
- [ ] Manual rollback rehearsal passes on macOS and Linux.
- [ ] Migration backup path is visible in logs/diagnostics.
- [ ] Release candidate report is produced.
- [ ] Release candidate tag is created.

Evidence:

- Release manifest.
- Checksums and signature output.
- Installer smoke notes.
- Rollback rehearsal notes.
- Known issues and unsupported downgrade list.

## Release Gate Summary

A release candidate cannot pass until these reports exist:

- [ ] Verified continuation report.
- [ ] Correctness report.
- [ ] Resource report.
- [ ] UX report.
- [ ] Security report.
- [ ] Release-candidate report.

Ship decision:

- [ ] All required capabilities pass.
- [ ] Verified continuation gate passes.
- [ ] Correctness gate passes.
- [ ] Resource gate passes.
- [ ] UX gate passes.
- [ ] Security gate passes.
- [ ] Release candidate artifacts are signed and reproducible enough for beta.
