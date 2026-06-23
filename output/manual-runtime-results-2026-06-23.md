# Manual Runtime Results - 2026-06-23

## Scope

- Checklist: `docs/manual-runtime-checklist.md`
- Requested UI driver: computer-use
- Result: computer-use was unavailable after 3 attempts with `runtime app is missing`; GUI checks used macOS Accessibility/AppleScript and Playwright fallback instead.

## Evidence

- Date/timezone: 2026-06-23, Asia/Seoul
- Git SHA: `126c6e917ade2a9e5f23de1bbf0c077b7574d203`
- macOS: 26.5.1 build 25F80
- Architecture: arm64
- rustc: 1.96.0 (ac68faa20 2026-05-25)
- node/npm: v26.0.0 / 11.12.1
- App tested: `target/release/bundle/macos/DevRelay.app`
- DMG tested: `target/release/bundle/dmg/DevRelay_0.1.0_aarch64.dmg`
- DEVRELAY_HOME tested: `/tmp/devrelay-manual.vlwdtq`
- Disposable repo: `/tmp/devrelay-repo.RY7UWC`
- Pre-existing dirty files: many existing worktree changes before this run; not modified by this report except generated build/output artifacts.

## Passed

- `npm run build:ui` passed and printed `UI source check passed`.
- `apps/desktop/dist/{index.html,app.css,app.js,favicon.svg}` exist.
- Prototype string scan found no matches in `apps/desktop/src` or `apps/desktop/dist`.
- `RUSTC_WRAPPER= cargo check -p devrelay-core -p devrelay-agent -p devrelay-desktop` passed.
- `RUSTC_WRAPPER= cargo test -p devrelay-agent -p devrelay-desktop` passed; agent tests: 14 passed.
- `RUSTC_WRAPPER= npm run build:dmg` built both app and DMG.
- `hdiutil verify` reported DMG checksum valid.
- `codesign --verify --deep --strict --verbose=2` reported app valid on disk and designated requirement satisfied.
- Browser fallback page title was `DevRelay Desktop`; runtime unavailable state was visible; project/device/run/activity counts were zero; favicon loaded with HTTP 200; browser console had 0 errors/warnings.
- Playwright overflow helper reported `violations: []` for desktop, short, and narrow viewports across Continue, Projects, Devices, Runs, Activity, and Settings.
- Tauri app without agent opened, did not crash, showed empty project states, and Refresh/Diagnostics remained clickable.
- Tauri app with foreground agent connected using `/tmp/devrelay-manual.vlwdtq`; app showed `Agent connected`, 1 device, and empty project state before registration.
- Agent logs showed RPCs for `rpc.negotiate`, `agent.health`, `projects.list`, `devices.list`, `runs.list`, `activity.list`, and `settings.get`.
- Disposable repo setup passed: one baseline commit, modified README, untracked notes.md, project id `manual-runtime`.
- `project add` and `projects list` succeeded through the agent and included `Manual Runtime Project`.
- Desktop refresh/event flow showed the project exactly once.
- Continue screen showed project name/path, active writer workspace, disabled unsupported handoff, and counts matching CLI status: staged 0, modified 1, untracked 1, conflicts 0.
- Status action called `status.get`; repeated clicks did not duplicate project rows.
- Checkpoint action called `checkpoint.create`, recover list showed a new snapshot, and dirty work stayed intact.
- Open folder opened Finder at `/private/tmp/devrelay-repo.RY7UWC/` without mutating repo state.
- Projects screen showed 1 registered row with workspace count 1 and matching change counts.
- Devices screen showed current device from `devices.list`, marked `This device`, with human-readable last-seen text.
- Runs screen showed the empty state and no fake scheduler/cache strings.
- Diagnostics export succeeded; output file existed at `/tmp/devrelay-manual.vlwdtq/diagnostics/diagnostics-1782223091.json`; it had `source_code_included: false`, `snapshot_objects_included: false`, and redacted local paths.
- CLI checkpoint for event test triggered app refresh/status calls without duplicate sidebar projects.
- Agent restart: app reconnected after agent returned and previous project state reappeared.
- DMG mounted at `/Volumes/DevRelay 2`; `DevRelay.app` and `icon.icns` were visible; volume detached cleanly.
- Missing project path safety: after repo rename, project remained listed, UI showed `Status error`, checkpoint failed, and recover list did not gain a new snapshot.
- Resource spot check: app 0.0% CPU / 120864 KB RSS; agent 0.0% CPU / 14976 KB RSS.

## Failed Or Risky

- computer-use could not start because its runtime app was missing, so the requested tool could not perform the GUI checks.
- Agent-off and missing-agent errors render the fixed text `The app did not receive a Tauri command bridge` even when the Tauri bridge works and the real problem is agent socket I/O. This is misleading.
- During agent-down state, the sidebar pill still said `Agent connected` while the main panel showed `Connection refused`.
- Checkpoint/event writes produced repeated agent warnings: `agent.ipc IPC write error error=I/O error: Invalid argument (os error 22)`.
- Activity UI stayed at `0 audit events` / `No activity` after successful checkpoint snapshots.
- Missing project path showed `Status error`, but the visible counts reset to 0/0/0/0, which may be confused with no changes even though it is not marked clean.

## Blocked Or Partial

- Live Settings UI: mDNS toggle saved and `settings.update` was called. Resource profile and editor command changes could not be driven reliably through AppleScript accessibility, so empty-editor rejection was not verified live.
- Keyboard smoke: Tab reached sidebar navigation and Refresh. Diagnostics/form-control traversal and Space/Enter activation were inconclusive with AppleScript focus collection.
- Full 5-minute RSS growth check was not run; only idle spot samples were recorded.
- DMG drag-copy to `/Applications` and copied-app launch were skipped because `/Applications/DevRelay.app` already existed and overwriting/removing it would affect user state.

## Artifacts

- Browser screenshots: `output/playwright/devrelay-desktop.png`, `output/playwright/devrelay-short.png`, `output/playwright/devrelay-narrow.png`
- Diagnostics export: `/tmp/devrelay-manual.vlwdtq/diagnostics/diagnostics-1782223091.json`
- Disposable home: `/tmp/devrelay-manual.vlwdtq`
- Disposable repo: `/tmp/devrelay-repo.RY7UWC`
