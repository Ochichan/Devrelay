# DevRelay VS Code Extension

This package is the first editor-context slice for DevRelay. It starts with a
small local-agent bridge, status bar state, command registration, and a manual
editor context capture command. Captured context is size-limited and sent to the
local agent through `editor.context.update`. `DevRelay: Restore Editor Context`
loads the latest context through `editor.context.latest`, opens saved folders
and files in recorded order where VS Code APIs allow, restores active
selections, breakpoints, and local dirty buffers, then reports
`editor.restore.ack` with partial details. Unsaved dirty buffers can be stored
locally in VS Code SecretStorage when `devrelay.captureUnsavedBuffers` is
enabled; untitled buffers require `devrelay.includeUntitledUnsavedBuffers`.
Restore opens dirty untitled documents and does not save them to disk. The
status bar reads agent leases and handoffs to show active, inactive, handoff,
and delayed protection states. Edit, save, and active-editor changes are sent to
the local agent through `editor.event.record`; meaningful edits increment source
generation and abort pending source handoffs.

Command Palette entries are grouped under DevRelay. Continue Here restores the
latest context, Continue Elsewhere captures context and starts a guarded
handoff when the local writer lease and a target device are available,
Checkpoint calls `checkpoint.create`, Open Recovery Timeline lists
`recover.list`, and Run Task opens recent `runs.list` history until an execution
RPC exists. Set `devrelay.captureEditorContext` to false to skip editor context
upload while still allowing the guarded handoff command to proceed.

```bash
npm install
npm run check
```
