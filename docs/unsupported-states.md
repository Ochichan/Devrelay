# Unsupported States

Last updated: 2026-06-24

Unsupported does not mean data may be discarded. It means DevRelay must block,
preserve as separate work, or ask for explicit user action.

## Product-Unsupported Today

- Production desktop UI.
- Editor context restore without real VS Code extension-host validation.
- Remote compute task execution.
- Windows UI through agent IPC.
- Windows named pipe local IPC.
- Windows pipe ACL enforcement.
- Packaged Windows background service.
- Native bootstrap environment hydration.
- Secret provider materialization.
- Hydration state machine visible in UI.
- Control API `/v1` endpoints.
- Signed release artifacts and update channels.

## Git States That Must Not Silently Degrade

- Interactive rebase reconstruction.
- Unknown sequencer states.
- Unsupported conflict operation variants.
- Missing LFS objects with no local sidecar fallback.
- Missing or corrupt CAS sidecar chunks.
- Unsafe target path portability conflicts.
- Symlink or reparse point materialization that could escape the workspace.

These states must block with recovery options or preserve work separately.

## User Work That Must Not Be Overwritten

- Dirty target workspaces.
- Inactive workspace edits.
- Stale lease publishes.
- Recovery targets with existing dirty content.
- Generated workspaces not owned by DevRelay.
- Any workspace whose path doctor reports a hard unsafe target issue.

## Execution That Must Not Happen Automatically

- Manifest bootstrap commands after their trust hash changes.
- Task commands on remote devices.
- Secret provider commands.
- User script providers.
- Code-changing agent tasks.

Every execution path above requires explicit trust approval and must run outside
the active canonical workspace unless the specific operation is a local
user-invoked workspace operation.
