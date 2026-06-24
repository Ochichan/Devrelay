# DevRelay VS Code Extension

This package is the first editor-context slice for DevRelay. It starts with a
small local-agent bridge, status bar state, command registration, and a manual
editor context capture command. Captured context is size-limited and sent to the
local agent through `editor.context.update`. Unsaved buffers and restore stay
behind later M8 checklist items.

```bash
npm install
npm run check
```
