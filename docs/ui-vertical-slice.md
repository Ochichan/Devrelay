# First UI Vertical Slice

Last updated: 2026-06-24

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

Decision: the first dogfood UI is macOS/Linux-first. A 3-OS first slice is
explicitly deferred until Windows IPC and startup behavior have the same
evidence level as macOS/Linux.

## Required Agent Commands And Events

The desktop slice may call only local agent APIs:

- `rpc.negotiate` to discover feature availability.
- `settings.get` and `settings.update` for local device identity and desktop
  preferences.
- `projects.list`, `projects.add`, and `project.status` for registered project
  state and source change summaries.
- `snapshots.list`, `checkpoint.create`, and `apply.snapshot` for checkpoint
  and target-apply state.
- `leases.list` for read-only active device state.
- `handoffs.list`, `handoff.begin`, `handoff.abort`,
  `handoff.target.verify`, `handoff.source.ready`, and `handoff.commit` for
  handoff state transitions.
- `devices.list`, `runs.list`, `activity.list`, `diagnostics.export`, and
  `events.subscribe` for secondary views and support evidence.

The UI state must update from initial RPC state plus event stream messages. The
first slice consumes:

- `snapshot.local.created`
- `snapshot.apply.verified`
- `handoff.state.changed`
- `security.blocked`
- `quota.warning`
- `events.subscribe` gap and reconnect state

The UI must never decide that work moved until agent verification and handoff
commit events say so.

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

## Failed Handoff Activity Details

A failed handoff Activity detail may include:

- user-safe title
- project display name or project ID
- source device display name
- target device display name
- phase label: saving state, preparing device, moving control, or opening
  project
- stable diagnostic code
- recommended safe action
- diagnostics bundle path when exported

It must not expose Git OIDs, CAS roots, lease epochs, certificate material, raw
policy names such as `snapshot-and-fork`, or unredacted local paths unless the
user explicitly exports diagnostics.

## Real-Device Dogfood Scripts

Clean target handoff:

1. Start the local agent on one macOS source and one Linux target on the same
   LAN.
2. Register the same disposable project on both devices.
3. Create a visible source edit on macOS.
4. Start handoff from the tray in two clicks.
5. Continue on Linux only after target preparation is visible.
6. Confirm the source still reports pending or moved state accurately.
7. Record Activity evidence for checkpoint creation, target verification, and
   handoff state changes.

Dirty target preservation:

1. Repeat the setup with a disposable project.
2. Add separate local work on the Linux target before continuing.
3. Start handoff from macOS.
4. On Linux, confirm the UI says the target has separate local work and nothing
   will be overwritten.
5. Choose preserve separately, open incoming work in a new folder, and cancel
   in separate runs.
6. After each run, verify the target's pre-existing local work remains intact.
7. Export diagnostics only after checking that the normal UI avoided internal
   policy names and Git object details.

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
