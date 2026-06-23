# First UI Vertical Slice

Last updated: 2026-06-23

The first desktop slice is not "build M7". It is one verified continuation path
from a real macOS source to a real Linux target.

## Product Goal

```text
Mac에서 작업하던 프로젝트를 Linux에서 두 번의 클릭으로 이어서 연다.
```

The slice proves:

- current work is protected
- the active writer is visible
- target readiness is visible
- nothing is overwritten
- verification gates the handoff
- the user can cancel or preserve separate work

## Platform Scope

Default first dogfood:

- macOS source
- Linux target
- local agent on both devices
- LAN or manually configured peer/anchor route

Windows is not part of the first slice unless Windows named pipe IPC, per-user
pipe ACL, and startup behavior are completed first.

## Required Surfaces

### Tray

```text
DevRelay
────────────────────────
payments-api
  Protected · 6s ago

Continue on
  ● Linux Workstation   Ready
  ○ Windows Desktop     Offline

Checkpoint now
Open DevRelay
```

### Continue Screen

```text
Continue your work

payments-api
feature/payment-retry

Active on
MacBook Pro

Latest checkpoint
8 seconds ago · Verified

Changes
3 modified · 1 staged · 2 untracked

[ Continue on Linux Workstation ]
[ Choose another device ]
```

### Handoff Dialog

```text
Moving payments-api to Linux Workstation

✓ Saving current work
✓ Verifying checkpoint
● Preparing Linux Workstation
○ Moving editing control
○ Opening project

Do not edit the project on MacBook until this completes.

[ Cancel safely ]
```

### Dirty Target Screen

```text
Linux Workstation has separate local work

Nothing will be overwritten.

Recommended
[ Preserve it as separate work and continue ]

Other options
[ Open your incoming work in a new folder ]
[ Cancel ]
```

Internal mapping:

- "Preserve it as separate work and continue" -> `snapshot-and-fork`
- "Open your incoming work in a new folder" -> `new-workspace`
- "Cancel" -> no mutation

Do not expose `snapshot-and-fork`, `lease`, `epoch`, `OID`, `CAS`, or
`canonical latest` in this screen.

### Completion Screen

```text
Ready on Linux Workstation

payments-api
feature/payment-retry

Code verified
Environment: Shell ready
Editor: Opening...

[ Open activity details ]
```

## Explicitly Out Of Scope

- Runs full screen.
- Scheduler controls.
- CAS details.
- Lease epoch.
- Git OID.
- Pairing certificate details.
- Nix store internals.
- Advanced retention settings.
- Graphs and statistics.
- VS Code context restore beyond a placeholder status.

## Agent Contract

The UI reads state from:

- initial status RPC
- project/device/session RPC
- event subscription
- command results

The UI must not:

- read Git directly
- scan the filesystem to compute canonical state
- infer writer authority locally
- apply dirty policies outside agent commands
- decide handoff success before verification events

## Completion Gate

- Same-LAN handoff starts from tray in two clicks.
- Clean target handoff completes on real macOS and Linux devices.
- Dirty target flow preserves target work and is understandable without Git
  terminology.
- Inactive source edit during handoff prevents stale lease transfer.
- Agent restart during handoff recovers to a clear state.
- Keyboard-only core flow works.
- Screen reader labels cover primary actions.
- Activity details expose diagnostic IDs for failures.
