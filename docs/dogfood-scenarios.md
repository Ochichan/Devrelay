# Dogfood Scenarios

Last updated: 2026-06-24

This document selects the real-device dogfood scenarios for M14.6. It is a
scenario plan, not completion evidence. Test result checkboxes remain open
until the runs are executed on real devices and defects are recorded.

## Two-Device Scenario

Goal:

```text
macOS source -> Linux target -> verified continuation
```

Roles:

- Source device: macOS laptop or desktop running the local agent and desktop
  app.
- Target device: Linux workstation running the local agent.
- Project: disposable Git project with a committed base, one tracked source
  edit, one staged change, and one safe untracked note.
- Discovery: same-LAN discovery when available; manual address fallback if mDNS
  is blocked.

Required runs:

- Clean target handoff from macOS to Linux.
- Dirty target preservation on Linux using preserve separately, new workspace,
  and cancel paths.
- Inactive edit on macOS during a pending handoff.
- Cold environment run where the target shell is not ready at handoff start.

Pass evidence:

- Handoff is started from the desktop/tray flow without requiring a Git command.
- The source checkpoint is visible before writer ownership moves.
- The target applies and verifies before handoff commit.
- Dirty target work is preserved as separate recoverable work or left unchanged
  in a new-workspace path.
- Activity details include stable diagnostic IDs without raw Git object IDs or
  local paths.

## Three-Device Scenario

Goal:

```text
macOS source -> Linux target, with a separate anchor device
```

Roles:

- Source device: macOS laptop or desktop.
- Target device: Linux workstation.
- Anchor device: always-on macOS or Linux machine with anchor mode enabled.
- Project: same disposable project shape as the two-device scenario, plus one
  large safe untracked file to exercise sidecar availability.

Required runs:

- Anchor-backed handoff while source and target are online.
- Anchor offline before handoff begins.
- Anchor offline after source checkpoint upload but before target apply.
- Peer-only fallback with documented limitations when anchor use is disabled.

Pass evidence:

- Anchor-backed transfer serves the snapshot after source upload completes.
- Anchor-offline failures are explicit and do not transfer writer ownership.
- Peer-only fallback reports source-required behavior when cached data is
  unavailable.
- Diagnostic bundle redaction remains enabled by default.

## Deferred Windows And WSL Scenario

Windows native and WSL are not part of the first three-device dogfood. Add that
scenario only after Windows named pipe IPC, pipe ACLs, and startup behavior are
credible enough for UI state authority.

Deferred shape:

```text
Windows native source -> Linux target -> WSL validation device
```

The WSL validation device must use its own Linux identity, its own
`DEVRELAY_HOME`, and a separate checkout on the distro filesystem.

## Defect Log Template

For each dogfood run, record:

- Date, timezone, and Git commit.
- Device role, OS version, architecture, and agent launch mode.
- Project fixture description.
- Scenario and step number.
- Expected result.
- Actual result.
- Whether user work was preserved.
- Diagnostic bundle path, if exported.
- Follow-up checklist item or issue.
