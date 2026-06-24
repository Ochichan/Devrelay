import assert from "node:assert/strict";
import test from "node:test";
import { restoreWorkspaceContext } from "../dist/contextRestore.js";

function selection(anchor, active) {
  return { anchor, active, start: anchor, end: active };
}

function capsule(overrides = {}) {
  const resourceA = { scheme: "file", path: "/repo/src/a.ts" };
  const resourceB = { scheme: "file", path: "/repo/src/b.ts" };
  return {
    schema_version: 1,
    source: "vscode",
    captured_at_unix_millis: 1000,
    workspace: {
      name: "repo",
      folders: [{ name: "repo", index: 0, path: "/repo" }],
    },
    tabs: [
      {
        view_column: 1,
        is_active: false,
        active_tab_index: 0,
        tabs: [
          {
            label: "a.ts",
            input_kind: "TabInputText",
            is_active: false,
            is_dirty: false,
            is_pinned: true,
            is_preview: false,
            resources: [resourceA],
          },
          {
            label: "b.ts",
            input_kind: "TabInputText",
            is_active: true,
            is_dirty: false,
            is_pinned: false,
            is_preview: false,
            resources: [resourceB],
          },
        ],
      },
      {
        view_column: 2,
        is_active: true,
        active_tab_index: 0,
        tabs: [
          {
            label: "a.ts",
            input_kind: "TabInputText",
            is_active: true,
            is_dirty: false,
            is_pinned: false,
            is_preview: false,
            resources: [resourceA],
          },
        ],
      },
    ],
    active_editor: {
      resource: resourceB,
      view_column: 1,
      cursor: { line: 3, character: 2 },
      selections: [selection({ line: 3, character: 2 }, { line: 4, character: 8 })],
    },
    breakpoints: [
      {
        resource: resourceA,
        line: 12,
        character: 4,
        enabled: true,
        condition: "count > 3",
      },
    ],
    terminals: [],
    limits: {
      max_workspace_folders: 8,
      max_tab_groups: 8,
      max_tabs: 64,
      max_breakpoints: 64,
      max_terminals: 16,
      max_selections: 16,
      max_string_chars: 4096,
      max_capsule_bytes: 131072,
      truncated: [],
    },
    ...overrides,
  };
}

test("restores workspace files active selections breakpoints and unsaved buffers", async () => {
  const openedWorkspaceFolders = [];
  const openedFiles = [];
  const restoredSelections = [];
  const restoredBreakpoints = [];
  const result = await restoreWorkspaceContext(capsule(), {
    openWorkspaceFolder: async (path) => {
      openedWorkspaceFolders.push(path);
    },
    openFile: async (resource, viewColumn) => {
      const editor = { key: resource.path ?? resource.uri, viewColumn };
      openedFiles.push(editor);
      return editor;
    },
    setSelections: (editor, selections) => {
      restoredSelections.push({ editor, selections });
    },
    addBreakpoints: async (breakpoints) => {
      restoredBreakpoints.push(...breakpoints);
      return breakpoints.length;
    },
    restoreUnsavedBuffers: async () => 2,
  });

  assert.equal(result.succeeded, true);
  assert.equal(result.partial, false);
  assert.deepEqual(openedWorkspaceFolders, ["/repo"]);
  assert.deepEqual(
    openedFiles.map((file) => [file.key, file.viewColumn]),
    [
      ["/repo/src/a.ts", 1],
      ["/repo/src/b.ts", 1],
    ]
  );
  assert.deepEqual(result.opened_files, ["/repo/src/a.ts", "/repo/src/b.ts"]);
  assert.equal(result.restored_active_file, "/repo/src/b.ts");
  assert.equal(result.restored_selections, 1);
  assert.equal(restoredSelections[0].editor.key, "/repo/src/b.ts");
  assert.equal(restoredSelections[0].selections[0].active.character, 8);
  assert.equal(result.restored_breakpoints, 1);
  assert.equal(restoredBreakpoints[0].condition, "count > 3");
  assert.equal(result.restored_unsaved_buffers, 2);
});

test("reports partial restore details without throwing", async () => {
  const broken = capsule({
    workspace: {
      folders: [{ name: "missing", index: 0, path: "/missing" }],
    },
    tabs: [
      {
        view_column: 1,
        is_active: true,
        tabs: [
          {
            label: "missing.ts",
            input_kind: "TabInputText",
            is_active: true,
            is_dirty: false,
            is_pinned: false,
            is_preview: false,
            resources: [{ scheme: "file", path: "/missing/src/missing.ts" }],
          },
        ],
      },
    ],
    active_editor: {
      resource: { scheme: "file", path: "/missing/src/missing.ts" },
      cursor: { line: 0, character: 0 },
      selections: [selection({ line: 0, character: 0 }, { line: 0, character: 0 })],
    },
    breakpoints: [
      {
        resource: { scheme: "file", path: "/missing/src/missing.ts" },
        line: 1,
        character: 0,
        enabled: true,
      },
    ],
  });

  const result = await restoreWorkspaceContext(broken, {
    openWorkspaceFolder: async () => {
      throw new Error("folder unavailable");
    },
    openFile: async () => {
      throw new Error("file unavailable");
    },
    setSelections: () => {
      throw new Error("selection unavailable");
    },
    addBreakpoints: async () => {
      throw new Error("breakpoints unavailable");
    },
    restoreUnsavedBuffers: async () => {
      throw new Error("unsaved unavailable");
    },
  });

  assert.equal(result.succeeded, false);
  assert.equal(result.partial, true);
  assert.equal(result.opened_files.length, 0);
  assert.match(result.partial_details.join("\n"), /workspace folder restore failed/);
  assert.match(result.partial_details.join("\n"), /file restore failed/);
  assert.match(result.partial_details.join("\n"), /active editor restore failed/);
  assert.match(result.partial_details.join("\n"), /breakpoint restore failed/);
  assert.match(result.partial_details.join("\n"), /unsaved buffer restore failed/);
});
