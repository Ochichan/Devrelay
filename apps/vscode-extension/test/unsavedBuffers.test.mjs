import assert from "node:assert/strict";
import test from "node:test";
import {
  captureUnsavedBuffers,
  loadUnsavedBufferCapsule,
  restoreUnsavedBufferCapsule,
  storeUnsavedBufferCapsule,
  unsavedBufferSummary,
} from "../dist/unsavedBuffers.js";

function uri(path, scheme = "file") {
  return {
    scheme,
    fsPath: scheme === "file" ? path : undefined,
    toString: () => `${scheme}:${path}`,
  };
}

function document({ path, text, dirty = true, untitled = false, language = "typescript" }) {
  return {
    uri: uri(path, untitled ? "untitled" : "file"),
    isDirty: dirty,
    isUntitled: untitled,
    languageId: language,
    version: 7,
    getText: () => text,
    save: () => {
      throw new Error("restore must not save documents");
    },
  };
}

test("captures dirty saved buffers and excludes untitled by default", () => {
  const capsule = captureUnsavedBuffers(
    [
      document({ path: "/repo/a.ts", text: "dirty" }),
      document({ path: "/repo/clean.ts", text: "clean", dirty: false }),
      document({ path: "Untitled-1", text: "draft", untitled: true }),
    ],
    { now: () => 10 }
  );

  assert.equal(capsule.local_only, true);
  assert.equal(capsule.storage, "vscode.SecretStorage");
  assert.equal(capsule.captured_at_unix_millis, 10);
  assert.equal(capsule.buffers.length, 1);
  assert.equal(capsule.buffers[0].text, "dirty");
  assert.deepEqual(
    capsule.excluded.map((entry) => entry.reason),
    ["clean", "untitled-disabled"]
  );
  assert.equal(unsavedBufferSummary(capsule), "1 dirty buffers, 2 excluded");
});

test("can include untitled buffers only when permitted", () => {
  const capsule = captureUnsavedBuffers(
    [document({ path: "Untitled-1", text: "draft", untitled: true })],
    { includeUntitled: true }
  );

  assert.equal(capsule.buffers.length, 1);
  assert.equal(capsule.buffers[0].is_untitled, true);
  assert.equal(capsule.buffers[0].scheme, "untitled");
});

test("applies buffer and total byte limits", () => {
  const capsule = captureUnsavedBuffers(
    [
      document({ path: "/repo/large.ts", text: "abcdef" }),
      document({ path: "/repo/second.ts", text: "1234" }),
    ],
    { maxBufferBytes: 5, maxTotalBytes: 3 }
  );

  assert.deepEqual(
    capsule.excluded.map((entry) => entry.reason),
    ["buffer-too-large", "total-too-large"]
  );
});

test("stores and restores buffers without saving to disk", async () => {
  const values = new Map();
  const secrets = {
    get: async (key) => values.get(key),
    store: async (key, value) => {
      values.set(key, value);
    },
    delete: async (key) => {
      values.delete(key);
    },
  };
  const capsule = captureUnsavedBuffers([document({ path: "/repo/a.ts", text: "dirty" })]);

  await storeUnsavedBufferCapsule(secrets, capsule);
  const loaded = await loadUnsavedBufferCapsule(secrets);

  const opened = [];
  const shown = [];
  const workspace = {
    openTextDocument: async (options) => {
      opened.push(options);
      return {
        isUntitled: true,
        isDirty: true,
        save: () => {
          throw new Error("restore must not save documents");
        },
      };
    },
  };
  const window = {
    showTextDocument: async (doc, options) => {
      shown.push({ doc, options });
      return {};
    },
  };

  const restored = await restoreUnsavedBufferCapsule(loaded, workspace, window);

  assert.equal(restored, 1);
  assert.deepEqual(opened, [{ language: "typescript", content: "dirty" }]);
  assert.equal(shown[0].doc.isUntitled, true);
  assert.equal(shown[0].doc.isDirty, true);
  assert.deepEqual(shown[0].options, { preview: false });
});
