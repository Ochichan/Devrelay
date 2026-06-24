# DevRelay VS Code Extension

This package is the first editor-context slice for DevRelay. It starts with a
small local-agent bridge, status bar state, command registration, and a manual
editor context capture command. Captured context is size-limited and sent to the
local agent through `editor.context.update`. Unsaved dirty buffers can be stored
locally in VS Code SecretStorage when `devrelay.captureUnsavedBuffers` is
enabled; untitled buffers require `devrelay.includeUntitledUnsavedBuffers`.
Restore opens dirty untitled documents and does not save them to disk.

```bash
npm install
npm run check
```
