import assert from "node:assert/strict";
import test from "node:test";
import {
  editorEventRecordParams,
  editorEventResultSummary,
  shouldNotifyEditorEvent,
  shouldWarnHandoffInProgress,
} from "../dist/editGuard.js";

function uri(path) {
  return {
    scheme: "file",
    fsPath: path,
    toString: () => `file://${path}`,
  };
}

test("builds text change event params with source-generation signal", () => {
  const params = editorEventRecordParams({
    eventKind: "text-document-changed",
    workspaceFolders: [{ uri: uri("/repo") }],
    document: {
      uri: uri("/repo/src/main.ts"),
      version: 12,
    },
    contentChangeCount: 1,
  });

  assert.deepEqual(params, {
    project: null,
    workspace_path: "/repo",
    event_kind: "text-document-changed",
    document_uri: "file:///repo/src/main.ts",
    document_path: "/repo/src/main.ts",
    document_version: 12,
    meaningful_edit: true,
  });
  assert.equal(shouldNotifyEditorEvent(params), true);
});

test("ignores empty text change events but not save or active-editor events", () => {
  const emptyChange = editorEventRecordParams({
    eventKind: "text-document-changed",
    contentChangeCount: 0,
  });
  const save = editorEventRecordParams({ eventKind: "text-document-saved" });
  const active = editorEventRecordParams({ eventKind: "active-editor-changed" });

  assert.equal(shouldNotifyEditorEvent(emptyChange), false);
  assert.equal(shouldNotifyEditorEvent(save), true);
  assert.equal(shouldNotifyEditorEvent(active), true);
});

test("warns when editing during handoff", () => {
  assert.equal(shouldWarnHandoffInProgress("handoff", "text-document-changed"), true);
  assert.equal(shouldWarnHandoffInProgress("active", "text-document-changed"), false);
  assert.equal(shouldWarnHandoffInProgress("handoff", "text-document-saved"), false);
});

test("summarizes mocked agent edit guard result", () => {
  const summary = editorEventResultSummary({
    project: "project-a",
    source_generation: 3,
    aborted_handoffs: [{ handoff_id: "ho_1", project_id: "project-a", state: "aborted" }],
  });

  assert.equal(
    summary,
    "editor event recorded for project-a; source generation 3; aborted 1 handoffs"
  );
});
