# Manual Runtime Checklist

Last updated: 2026-06-24

This checklist is the manual gate for the desktop runtime slice. It is not a
replacement for automated tests. It catches product failures that are hard to
see from unit tests: stale UI state, unclear agent errors, broken packaging,
scroll traps, missing runtime permissions, and unsafe-looking recovery paths.

For the full manual evidence map across real-device dogfood, security,
resource, packaging, rollback, and release gates, start with
[manual-verification-runbook.md](manual-verification-runbook.md). This document
is the desktop runtime sub-runbook.

Run this checklist against a disposable repository first. Do not use a real work
repository until every destructive-looking path below has passed on a throwaway
workspace.

## Evidence Header

Record this before testing:

- [ ] Date and local timezone.
- [ ] Git commit SHA.
- [ ] macOS version and architecture.
- [ ] `rustc --version`.
- [ ] `node --version` and `npm --version`.
- [ ] App path tested.
- [ ] DMG path tested.
- [ ] `DEVRELAY_HOME` path, or "default user home" if unset.
- [ ] Whether the agent was launched from cargo, a release binary, or an OS
  service.
- [ ] Screen sizes tested.
- [ ] Any known pre-existing dirty files in the DevRelay repo.

Useful commands:

```bash
git rev-parse HEAD
sw_vers
uname -m
rustc --version
node --version
npm --version
```

## Build And Packaging

### manual/runtime/build-001 - UI Build

```bash
cd apps/desktop
npm run build:ui
```

Pass:

- [ ] `apps/desktop/dist/index.html` exists.
- [ ] `apps/desktop/dist/app.css` exists.
- [ ] `apps/desktop/dist/app.js` exists.
- [ ] `apps/desktop/dist/favicon.svg` exists.
- [ ] `npm run check:ui` reports `UI source check passed`.
- [ ] No old prototype strings are present in `apps/desktop/src` or
  `apps/desktop/dist`.

Check:

```bash
rg -n "payments-api|Mac Studio|Linux Workstation|Windows / WSL|web-console|mobile-shell|legacy-admin|data-pipeline|feature/refund|Ryzen|RTX|M4 Max|scheduler score|Nix cache 97|9d3fa82|41\\.2s" apps/desktop/src apps/desktop/dist
```

Expected: no matches.

### manual/runtime/build-002 - Rust Check

```bash
RUSTC_WRAPPER= cargo check -p devrelay-core -p devrelay-agent -p devrelay-desktop
```

Pass:

- [ ] Check completes with no errors.
- [ ] No new warnings from `devrelay-desktop`.

### manual/runtime/build-003 - Agent And Desktop Tests

```bash
RUSTC_WRAPPER= cargo test -p devrelay-agent -p devrelay-desktop
```

Pass:

- [ ] Agent integration tests pass.
- [ ] Desktop crate tests pass.
- [ ] The desktop bootstrap RPC test covers `devices.list`, `activity.list`,
  `runs.list`, `settings.get`, and `settings.update`.

Known caveat:

- Full `cargo test -p devrelay-core -p devrelay-agent -p devrelay-desktop`
  currently depends on pre-existing core test fixture state in
  `crates/devrelay-core/src/environment.rs`. If it fails there, record the exact
  failure separately from desktop runtime results.

### manual/runtime/build-004 - App And DMG Build

```bash
cd apps/desktop
RUSTC_WRAPPER= npm run build:dmg
```

Pass:

- [ ] App exists at `target/release/bundle/macos/DevRelay.app`.
- [ ] DMG exists at `target/release/bundle/dmg/DevRelay_0.1.0_aarch64.dmg`.
- [ ] Build output shows `UI source check passed`.
- [ ] Build output shows both bundles finished.

### manual/runtime/build-005 - Bundle Verification

```bash
hdiutil verify target/release/bundle/dmg/DevRelay_0.1.0_aarch64.dmg
codesign --verify --deep --strict --verbose=2 target/release/bundle/macos/DevRelay.app
```

Pass:

- [ ] `hdiutil` reports the DMG checksum is valid.
- [ ] `codesign` reports the app is valid on disk.
- [ ] `codesign` reports the designated requirement is satisfied.

## Runtime Modes

### manual/runtime/mode-001 - Browser Fallback Mode

This checks that the static frontend does not fake healthy data when Tauri is
absent.

```bash
python3 -m http.server 4173 --bind 127.0.0.1 --directory apps/desktop/dist
```

Open `http://127.0.0.1:4173`.

Pass:

- [ ] Page title is `DevRelay Desktop`.
- [ ] UI shows runtime unavailable.
- [ ] Project, device, run, and activity counts are zero.
- [ ] No fake project or device names appear.
- [ ] Browser console has no app JavaScript errors.
- [ ] `favicon.svg` loads without a 404.

Cleanup:

```bash
pkill -f "python3 -m http.server 4173" || true
```

### manual/runtime/mode-002 - Tauri App Without Agent

Launch the app while no DevRelay agent is running.

```bash
pkill -f devrelay-agent || true
open -n target/release/bundle/macos/DevRelay.app
```

Pass:

- [ ] App window opens.
- [ ] UI does not crash or stay on the loading screen.
- [ ] Agent state is visibly unavailable.
- [ ] Empty project/device/run/activity states are shown.
- [ ] Refresh remains clickable and does not crash the app.
- [ ] Diagnostics action reports a clear failure instead of a silent no-op.
- [ ] No fake data appears.

Cleanup:

```bash
pkill -f devrelay-desktop || true
```

### manual/runtime/mode-003 - Tauri App With Foreground Agent

Use the default user home for Finder/open testing, or run the app binary
directly with `DEVRELAY_HOME` for an isolated home.

Default-home mode:

```bash
RUSTC_WRAPPER= cargo run -p devrelay-agent -- --foreground --log-level debug
```

In another terminal:

```bash
open -n target/release/bundle/macos/DevRelay.app
```

Isolated-home mode:

```bash
export DEVRELAY_HOME="$(mktemp -d /tmp/devrelay-manual.XXXXXX)"
RUSTC_WRAPPER= cargo run -p devrelay-agent -- --foreground --log-level debug
```

In another terminal:

```bash
DEVRELAY_HOME="$DEVRELAY_HOME" target/release/bundle/macos/DevRelay.app/Contents/MacOS/devrelay-desktop
```

Pass:

- [ ] App shows agent connected.
- [ ] Runtime path points to the same `DEVRELAY_HOME` as the agent.
- [ ] Refresh calls the agent and leaves the app usable.
- [ ] Agent logs show JSON-RPC requests for `rpc.negotiate`, `agent.health`,
  `projects.list`, `devices.list`, `runs.list`, `activity.list`, and
  `settings.get`.

## Disposable Repository Setup

### manual/runtime/repo-001 - Create Test Repository

Use a new disposable repository:

```bash
export DEVRELAY_MANUAL_REPO="$(mktemp -d /tmp/devrelay-repo.XXXXXX)"
cd "$DEVRELAY_MANUAL_REPO"
git init -b main
git config user.email "devrelay@example.test"
git config user.name "DevRelay Manual"
cat > README.md <<'EOF'
base
EOF
cat > devrelay.toml <<'EOF'
schema = 1
project_id = "manual-runtime"
name = "Manual Runtime Project"

[workspace]
untracked = "safe"
portable_paths = "strict"
EOF
git add README.md devrelay.toml
git commit -m "base"
printf "base\nchanged\n" > README.md
printf "untracked\n" > notes.md
```

Pass:

- [ ] Repository has one committed baseline.
- [ ] `README.md` is modified.
- [ ] `notes.md` is untracked.
- [ ] `devrelay.toml` has project id `manual-runtime`.

### manual/runtime/repo-002 - Register Project Through Agent

Run while the foreground agent is active:

```bash
cd /path/to/Devrelay
RUSTC_WRAPPER= cargo run -p devrelay-cli -- project add "$DEVRELAY_MANUAL_REPO" --manifest "$DEVRELAY_MANUAL_REPO/devrelay.toml" --json
RUSTC_WRAPPER= cargo run -p devrelay-cli -- projects list --json
```

Pass:

- [ ] `project add` succeeds through the agent.
- [ ] `projects list` includes `Manual Runtime Project`.
- [ ] Desktop refresh shows the project in the sidebar.
- [ ] Continue screen selects the project or can select it from the sidebar.

## Desktop UI Checks

### manual/runtime/ui-001 - Initial Bootstrap State

Pass:

- [ ] Sidebar brand shows the configured fabric name, not a prototype string.
- [ ] Agent pill shows connected when the agent is running.
- [ ] Project count equals the agent project registry count.
- [ ] Device count equals `devices.list` output.
- [ ] Runs count equals `runs.list` output.
- [ ] Activity count equals `activity.list` output.
- [ ] Long paths wrap or scroll; they do not overlap adjacent content.

### manual/runtime/ui-002 - Continue Screen

Pass:

- [ ] Current project name comes from `projects.list`.
- [ ] Project path comes from `projects.list`.
- [ ] Writer/workspace section comes from project workspace metadata.
- [ ] Status button calls `project_status`.
- [ ] Modified/staged/untracked/conflict counts match `git status`.
- [ ] If status fails, the error is visible and the rest of the app remains
  usable.
- [ ] "Continue on another device" does not fake a handoff before the UI has
  target apply and verification wiring.
- [ ] Unsupported handoff state is explicit and disabled.

### manual/runtime/ui-003 - Project Status Action

Click `Status` on the Continue screen and in the Projects table.

Pass:

- [ ] Status loading state is visible.
- [ ] Counts update after the command returns.
- [ ] Repeated clicks do not duplicate project rows.
- [ ] Long branch names and paths remain readable.

CLI cross-check:

```bash
RUSTC_WRAPPER= cargo run -p devrelay-cli -- status --repo "$DEVRELAY_MANUAL_REPO" --manifest "$DEVRELAY_MANUAL_REPO/devrelay.toml" --json
```

### manual/runtime/ui-004 - Checkpoint Action

Click `Checkpoint now`.

Pass:

- [ ] Button disables or shows operation state while running.
- [ ] Success toast appears.
- [ ] App refreshes after success.
- [ ] Agent logs show `checkpoint.create`.
- [ ] Activity or snapshot evidence is visible after refresh if the agent
  recorded it.
- [ ] The app does not claim cross-device handoff completion.

CLI cross-check:

```bash
RUSTC_WRAPPER= cargo run -p devrelay-cli -- recover list --json
```

### manual/runtime/ui-005 - Open Project Action

Click `Open folder`.

Pass:

- [ ] macOS opens the project folder.
- [ ] The app remains focused or returns usable after the OS action.
- [ ] Failure to open a missing path is reported as a visible error.
- [ ] The action never scans Git or mutates repository state.

### manual/runtime/ui-006 - Projects Screen

Pass:

- [ ] Empty state appears when no project is registered.
- [ ] Registered projects appear exactly once.
- [ ] Workspace count matches project metadata.
- [ ] `Continue` selects the project and navigates to Continue.
- [ ] Table scrolls horizontally when columns exceed narrow width.
- [ ] No page-level horizontal overflow appears.

### manual/runtime/ui-007 - Devices Screen

Pass:

- [ ] Empty state appears with no device records.
- [ ] Device records come from `devices.list`.
- [ ] Current device is marked only when its id matches settings.
- [ ] Long device names and platform keys wrap or scroll without overlap.
- [ ] Last-seen values are human-readable.

### manual/runtime/ui-008 - Runs Screen

Pass:

- [ ] Empty state appears with no task runs.
- [ ] Run records come from `runs.list`.
- [ ] Long commands remain readable in the table.
- [ ] Metadata JSON is scrollable.
- [ ] No scheduler scores or fake cache stats appear.

### manual/runtime/ui-009 - Activity Screen

Pass:

- [ ] Empty state appears with no audit events.
- [ ] Events come from `activity.list`.
- [ ] Outcome badges do not rely only on color.
- [ ] Long summaries and JSON details do not overlap.
- [ ] Activity panel scrolls independently when event count is high.

### manual/runtime/ui-010 - Settings Screen

Change resource profile, editor command, and mDNS toggle.

Pass:

- [ ] Current values come from `settings.get`.
- [ ] Save calls `settings.update`.
- [ ] Successful save shows a toast.
- [ ] App refreshes and persists updated values.
- [ ] Empty editor command is rejected by the agent and shown as an error.
- [ ] Runtime socket and home paths are visible and wrap safely.

## Event Stream And Refresh

### manual/runtime/events-001 - Event Bridge Connect

With the app open and agent running, perform a checkpoint from CLI:

```bash
RUSTC_WRAPPER= cargo run -p devrelay-cli -- checkpoint --repo "$DEVRELAY_MANUAL_REPO" --manifest "$DEVRELAY_MANUAL_REPO/devrelay.toml" --label "manual event" --json
```

Pass:

- [ ] App receives an agent event or refreshes shortly after.
- [ ] No duplicate sidebar projects appear.
- [ ] Activity/status remains internally consistent after refresh.
- [ ] Agent reconnect after app idle does not force a reload loop.

### manual/runtime/events-002 - Agent Restart

Kill and restart the foreground agent while the app is open.

Pass:

- [ ] App shows disconnected or degraded state without crashing.
- [ ] App reconnects after the agent returns.
- [ ] Refresh works after reconnect.
- [ ] Previously registered projects reappear.
- [ ] No stale operation spinner remains.

## Overflow And Accessibility

### manual/runtime/layout-001 - Viewport Matrix

Test at least:

- [ ] 1280 x 820.
- [ ] 1024 x 560.
- [ ] 390 x 740.

For every view:

- [ ] Main content scrolls when taller than viewport.
- [ ] Sidebar scrolls when its content is taller than viewport.
- [ ] Tables scroll horizontally inside table containers.
- [ ] JSON metadata boxes scroll internally.
- [ ] Buttons do not clip text.
- [ ] Long paths do not overlap badges or adjacent columns.
- [ ] There is no document-level horizontal overflow.

Automated helper:

```bash
python3 -m http.server 4173 --bind 127.0.0.1 --directory apps/desktop/dist
export CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
export PWCLI="$CODEX_HOME/skills/playwright/scripts/playwright_cli.sh"
"$PWCLI" open http://127.0.0.1:4173
"$PWCLI" run-code --filename output/playwright/overflow-check.js
```

Pass:

- [ ] All report entries have `violations: []`.
- [ ] Command calls include `checkpoint_create`, `open_project`,
  `diagnostics_export`, and `settings_update`.

### manual/runtime/a11y-001 - Keyboard Smoke

Pass:

- [ ] Tab reaches sidebar navigation.
- [ ] Tab reaches Refresh and Diagnostics.
- [ ] Tab reaches Continue screen actions.
- [ ] Enter or Space activates focused buttons.
- [ ] Focus outline is visible.
- [ ] Disabled handoff controls are not presented as active actions.

## Diagnostics And Logs

### manual/runtime/diagnostics-001 - Diagnostics Export

Click Diagnostics in the app.

Pass:

- [ ] Success or failure toast appears.
- [ ] On success, exported path exists.
- [ ] Diagnostic bundle does not include raw source files.
- [ ] Diagnostic bundle does not include snapshot Git objects.
- [ ] Local paths are redacted by default where diagnostics promise redaction.

CLI cross-check:

```bash
RUSTC_WRAPPER= cargo run -p devrelay-cli -- diagnostics export --json
```

### manual/runtime/logs-001 - Runtime Logs

Pass:

- [ ] Agent logs include JSON-RPC request IDs.
- [ ] Errors are actionable and name the failed method.
- [ ] App does not log repeated reconnect errors faster than the retry interval.
- [ ] No secret values from `devrelay.toml` or environment files appear in logs.

## DMG Install Path

### manual/runtime/dmg-001 - Mount DMG

```bash
hdiutil attach target/release/bundle/dmg/DevRelay_0.1.0_aarch64.dmg
```

Pass:

- [ ] DMG mounts.
- [ ] `DevRelay.app` is visible in the mounted volume.
- [ ] App icon renders.
- [ ] Drag-copy to `/Applications` succeeds.

### manual/runtime/dmg-002 - Launch Copied App

Launch the copied app from Finder or:

```bash
open -n /Applications/DevRelay.app
```

Pass:

- [ ] App launches.
- [ ] macOS does not report a damaged app.
- [ ] Unsigned/local signing warning is understood for development builds.
- [ ] App uses the default user `DEVRELAY_HOME`.
- [ ] Agent connection works when the default-home agent is running.

Cleanup:

```bash
hdiutil detach /Volumes/DevRelay || true
rm -rf /Applications/DevRelay.app
```

## Safety Smoke Checks

These are manual smoke checks only. They do not replace safety integration
suites.

### manual/runtime/safety-001 - Dirty Work Is Visible

With modified and untracked files in the disposable repo:

- [ ] Continue screen shows non-zero local change counts.
- [ ] Checkpoint does not erase local changes.
- [ ] Open folder does not mutate local changes.
- [ ] Unsupported handoff does not claim that work moved elsewhere.

### manual/runtime/safety-002 - Missing Project Path

Move or rename the disposable repo after registration, then refresh/status.

Pass:

- [ ] UI reports status/open failure clearly.
- [ ] Project remains listed.
- [ ] No fake clean status appears.
- [ ] Checkpoint failure does not create a success toast.

### manual/runtime/safety-003 - Inactive Or Unsupported Remote Work

Until real handoff UI adapter and target apply wiring exist:

- [ ] Remote continuation controls remain disabled.
- [ ] UI explains that remote handoff is not available from this build.
- [ ] No target device is marked as writer because of a local UI guess.
- [ ] No lease, epoch, Git OID, CAS, or certificate details are shown in the
  first-slice UI.

## Performance Smoke

### manual/runtime/perf-001 - Idle App And Agent

With one registered disposable project and no operation running:

- [ ] App CPU settles near idle after initial bootstrap.
- [ ] Agent CPU settles near idle after initial status/checkpoint work.
- [ ] No continuous growth in app RSS over 5 minutes.
- [ ] No continuous growth in agent RSS over 5 minutes.
- [ ] Agent does not spam logs while idle.

Suggested commands:

```bash
ps -o pid,pcpu,rss,command -p "$(pgrep -n devrelay-desktop)"
ps -o pid,pcpu,rss,command -p "$(pgrep -n devrelay-agent)"
```

For release evidence, use the repeatable benchmark plan in
[resource-benchmark.md](resource-benchmark.md).

## Failure Recording

For every failed check, record:

- [ ] Check id.
- [ ] Exact expected result.
- [ ] Exact actual result.
- [ ] Screenshot or terminal output.
- [ ] Agent log excerpt with request id if available.
- [ ] Whether retry/restart changes the result.
- [ ] Whether data was preserved.
- [ ] Proposed owner: core, agent, desktop UI, packaging, docs, or unknown.

## Exit Criteria

The desktop runtime is acceptable for local dogfood only when:

- [ ] Build, package, app launch, and DMG mount checks pass.
- [ ] Agent-off and agent-on runtime modes are understandable.
- [ ] Disposable project registration, status, checkpoint, diagnostics, and
  settings paths work.
- [ ] Every screen has a usable empty state.
- [ ] Every screen has a usable error state.
- [ ] Every screen has a usable ready state.
- [ ] Overflow checks pass on desktop, short, and narrow viewports.
- [ ] Unsupported handoff behavior is explicit and disabled.
- [ ] No fake prototype data appears.
- [ ] No data-loss-looking behavior occurs in the disposable repository.
