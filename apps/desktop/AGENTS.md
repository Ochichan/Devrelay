# DevRelay Desktop Frontend Guide

Scope: `apps/desktop` (Tauri shell + `src/` frontend). This file is the
contract for anyone (human or agent) editing the desktop UI.

## Architecture

- `src/app.js` is a single-file, framework-free frontend. It renders the whole
  shell with `render()` into `#app`, keeps all state in the top-level `state`
  object, and re-renders on every mutation. Keep it a single file: the
  behavioral check runs it inside a Node `vm` sandbox and drives the globals
  `state`, `render`, `handleAction`, and `validateSettingsInput` directly.
- The local agent is the only state authority. The UI may only:
  - invoke the whitelisted Tauri commands (see `allowedCommands` in
    `scripts/check-ui-state-authority.mjs`),
  - consume the `devrelay-agent-*` and `devrelay-tray-*` events,
  - hold presentation-only state (active view, dialogs, toasts, theme).
  Never read Git, the filesystem, shells, or durable browser storage, and
  never declare handoff success before agent verification events.
- The `vm` sandbox has no `setTimeout`, `matchMedia`, `location`, or
  `document.documentElement` globals: always call `window.setTimeout`, guard
  optional browser APIs with `?.`, and touch only `document.querySelector`,
  `document.querySelectorAll`, `document.addEventListener`, and
  `document.activeElement`.

## Verification

Run from `apps/desktop`:

```bash
npm run build:ui
```

This runs `prepare-ui` plus four gates, all of which must stay green:

1. `check-ui-source.mjs` - bans prototype demo strings, requires the visual
   direction markers in `app.css`.
2. `check-ui-state-authority.mjs` - command whitelist, required event
   listeners, forbidden state sources.
3. `check-feature-status.mjs` - keeps the not-built registry and its markers
   in sync (see below).
4. `check-event-bridge.mjs` - boots `src/app.js` in a VM with fixture agent
   data and asserts rendered behavior for every view, the handoff dialog,
   events, and copy safety.

## Not built yet markers (read this before finishing a feature)

The frontend is built to the complete-product design. Anything the local
agent cannot back yet is NOT simulated; it is declared once in the `FEATURES`
registry in `src/app.js` (between `FEATURE-REGISTRY-START` and
`FEATURE-REGISTRY-END`) and rendered through the standard helpers:

- `pendingChip(id)` - the visible "Not built yet" chip.
- `pendingPanel(id, copy)` - a stub panel where a whole section is pending.
- `pendingAction(id, label, ...)` - a button that only shows the
  "... is not wired to the agent yet" toast via `data-action="feature-pending"`.

When you complete one of these features (the agent RPC and, if needed, a new
Tauri command in `src-tauri/src/main.rs` exist):

1. Wire the real UI behavior. If you added a Tauri command, extend
   `allowedCommands` in `scripts/check-ui-state-authority.mjs` and update
   `docs/api-surface.md` first (see CONTRIBUTING change discipline).
2. Delete the feature's entry from `FEATURES` AND every one of its markers:
   the `pendingChip`/`pendingPanel`/`pendingAction` call sites and any
   `data-feature`/`data-feature-pending` references for that id.
3. Replace the feature's pending-marker assertions in
   `scripts/check-event-bridge.mjs` with assertions on the live behavior.
4. Run `npm run build:ui`. `check-feature-status.mjs` fails if the registry
   and the markers drift, so a completed feature cannot silently keep its
   placeholder.
5. Update the M7 notes in `docs/current-state.md` when the change is
   milestone-visible.

Do not add new fake data, simulated progress, or hidden half-features: a new
unfinished surface gets a new `FEATURES` entry plus markers, nothing else.

## Copy rules

- UI copy is English, plain, and free of internal wire terminology. The
  event-bridge check scans the rendered HTML of the handoff flow with a
  case-insensitive substring ban, so avoid even words that contain the banned
  fragments (`lease` also matches "release"/"please"; `OID` matches "avoid"/
  "void"; `CAS` matches "case"/"cascade"; plus `epoch`, `snapshot-and-fork`,
  `new-workspace`, `canonical latest`). Class names and data attributes count
  because the scan runs over `innerHTML`.
- Never render internal identifiers (lease ids, OIDs, state hashes, audit
  detail payloads). They belong only in exported diagnostics.
- Dirty-target choices stay in user language: preserve separately, open in a
  new folder, or cancel safely.

## Related docs

- `docs/ui-vertical-slice.md` - first-slice product scope and agent contract.
- `docs/api-surface.md` - UI boundary and RPC review triggers.
- `docs/current-state.md` - milestone status including M7 desktop UX.
