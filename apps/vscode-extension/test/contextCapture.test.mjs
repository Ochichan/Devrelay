import assert from "node:assert/strict";
import test from "node:test";
import {
  assertContextWithinLimit,
  captureWorkspaceContext,
  contextSummary,
  editorContextUpdateParams,
} from "../dist/contextCapture.js";

function fileUri(path) {
  return {
    scheme: "file",
    fsPath: path,
    toString: () => `file://${path}`,
  };
}

function position(line, character) {
  return { line, character };
}

function selection(anchor, active) {
  return {
    anchor,
    active,
    start: anchor,
    end: active,
  };
}

test("captures VS Code workspace context from public API shape", () => {
  const tab = {
    label: "main.ts",
    input: { uri: fileUri("/repo/src/main.ts") },
    isActive: true,
    isDirty: false,
    isPinned: true,
    isPreview: false,
  };
  const api = {
    workspace: {
      name: "Demo",
      workspaceFolders: [{ name: "repo", index: 0, uri: fileUri("/repo") }],
    },
    window: {
      activeTextEditor: {
        document: { uri: fileUri("/repo/src/main.ts") },
        viewColumn: 1,
        selection: selection(position(10, 4), position(10, 8)),
        selections: [selection(position(10, 4), position(10, 8))],
      },
      tabGroups: {
        all: [
          {
            viewColumn: 1,
            isActive: true,
            activeTab: tab,
            tabs: [tab],
          },
        ],
      },
      terminals: [
        {
          name: "zsh",
          creationOptions: { cwd: fileUri("/repo") },
          state: { isInteractedWith: true },
        },
      ],
    },
    debug: {
      breakpoints: [
        {
          enabled: true,
          condition: "count > 3",
          location: {
            uri: fileUri("/repo/src/main.ts"),
            range: { start: position(12, 2), end: position(12, 2) },
          },
        },
      ],
    },
  };

  const capsule = captureWorkspaceContext(api, { now: () => 1234 });

  assert.equal(capsule.schema_version, 1);
  assert.equal(capsule.source, "vscode");
  assert.equal(capsule.captured_at_unix_millis, 1234);
  assert.equal(capsule.workspace.folders[0].path, "/repo");
  assert.equal(capsule.tabs[0].active_tab_index, 0);
  assert.equal(capsule.tabs[0].tabs[0].resources[0].path, "/repo/src/main.ts");
  assert.equal(capsule.active_editor?.cursor.line, 10);
  assert.equal(capsule.active_editor?.selections[0].active.character, 8);
  assert.equal(capsule.breakpoints[0].line, 12);
  assert.equal(capsule.terminals[0].title, "zsh");
  assert.equal(capsule.terminals[0].cwd?.path, "/repo");
  assert.equal(contextSummary(capsule), "1 folders, 1 tabs, 1 breakpoints, 1 terminals");

  const params = editorContextUpdateParams(capsule);
  assert.equal(params.workspace_path, "/repo");
  assert.equal(params.project, null);
  assert.equal(params.capsule, capsule);
});

test("applies capture count limits and enforces byte limit", () => {
  const api = {
    workspace: {
      workspaceFolders: [
        { name: "a", index: 0, uri: fileUri("/a") },
        { name: "b", index: 1, uri: fileUri("/b") },
      ],
    },
    window: {
      tabGroups: { all: [] },
      terminals: [],
    },
    debug: { breakpoints: [] },
  };

  const capsule = captureWorkspaceContext(api, {
    now: () => 1,
    limits: { max_workspace_folders: 1 },
  });

  assert.equal(capsule.workspace.folders.length, 1);
  assert.deepEqual(capsule.limits.truncated, ["workspace.folders:2->1"]);
  assert.throws(
    () => assertContextWithinLimit(capsule, { max_capsule_bytes: 10 }),
    /Editor context capsule is \d+ bytes; limit is 10 bytes/
  );
});
