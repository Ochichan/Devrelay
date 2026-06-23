# Recovery Policy

Last updated: 2026-06-23

Recovery is a primary product behavior, not an exception path. It must be
non-destructive by default.

## Defaults

- Open recovery into a new workspace unless the target is known clean.
- Preserve dirty target work before applying incoming work.
- Pin dirty-target backups by default.
- Keep stale and inactive work as separate sessions.
- Keep recovery copy clear in UI language: "separate work", "new folder", or
  "recovered workspace".

## Recovery Sources

DevRelay may recover from:

- local snapshot store
- anchor snapshot bare repo
- anchor CAS sidecars
- pinned dirty-target backup snapshots
- fork sessions created from inactive edits
- handoff journal records

## User-Facing Recovery Actions

The first product UI should expose only:

- Open incoming work in a new folder.
- Preserve this device's work separately and continue.
- Cancel.
- Open activity details.

Advanced concepts such as snapshot ID, lease epoch, Git OID, CAS chunk, and
route decision belong in Activity or diagnostics.

## Handoff Recovery

If the agent crashes during handoff:

- before lease commit: recover or abort without changing the holder
  incorrectly
- after lease commit: replay idempotently and report the committed holder
- after expiration: abort and leave a clear recovery record

The user must never need to infer the correct writer from raw Git state.

## Diagnostic Recovery Evidence

Diagnostics should include:

- operation ID
- handoff ID
- phase history
- source and target device IDs
- redacted project/workspace identity
- verification result
- safe actions

Diagnostics must exclude source code, snapshot objects, and raw secret values by
default.
