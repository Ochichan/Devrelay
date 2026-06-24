import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const packageJson = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8"));
const extensionSource = await readFile(new URL("../src/extension.ts", import.meta.url), "utf8");

test("package exposes the VS Code extension entrypoint", () => {
  assert.equal(packageJson.main, "./dist/extension.js");
  assert.equal(packageJson.engines.vscode, "^1.90.0");
  assert.deepEqual(packageJson.activationEvents, [
    "onStartupFinished",
    "onCommand:devrelay.refreshConnection",
    "onCommand:devrelay.explainState",
    "onCommand:devrelay.continueHere",
    "onCommand:devrelay.continueElsewhere",
    "onCommand:devrelay.captureContext",
    "onCommand:devrelay.restoreContext",
    "onCommand:devrelay.createCheckpoint",
    "onCommand:devrelay.runTask",
    "onCommand:devrelay.openRecoveryTimeline",
    "onCommand:devrelay.captureUnsavedBuffers",
    "onCommand:devrelay.restoreUnsavedBuffers",
    "onCommand:devrelay.openDashboard",
  ]);
});

test("contributed commands are registered by the extension", () => {
  const commands = packageJson.contributes.commands.map((command) => command.command);
  assert.deepEqual(commands, [
    "devrelay.refreshConnection",
    "devrelay.explainState",
    "devrelay.continueHere",
    "devrelay.continueElsewhere",
    "devrelay.captureContext",
    "devrelay.restoreContext",
    "devrelay.createCheckpoint",
    "devrelay.runTask",
    "devrelay.openRecoveryTimeline",
    "devrelay.captureUnsavedBuffers",
    "devrelay.restoreUnsavedBuffers",
    "devrelay.openDashboard",
  ]);

  for (const command of commands) {
    assert.match(extensionSource, new RegExp(`registerCommand\\("${command}"`));
  }
  assert.equal(
    packageJson.contributes.commands.every((command) => command.category === "DevRelay"),
    true
  );
});

test("extension surfaces local agent connection state", () => {
  assert.match(extensionSource, /createStatusBarItem/);
  assert.match(extensionSource, /client\.call<AgentHealthResult>\("agent\.health"\)/);
  assert.match(extensionSource, /client\.call<EditorContextUpdateResult>\(\s*"editor\.context\.update"/);
  assert.match(extensionSource, /client\.call<EditorContextLatestResult>\("editor\.context\.latest"/);
  assert.match(extensionSource, /client\.call<EditorRestoreAckResult>\("editor\.restore\.ack"/);
  assert.match(extensionSource, /client\.call<CheckpointCreateResult>\("checkpoint\.create"/);
  assert.match(extensionSource, /client\.call<RecoverListResult>\("recover\.list"/);
  assert.match(extensionSource, /client\.call<RunsListResult>\("runs\.list"/);
  assert.match(extensionSource, /client\.call<HandoffMutationResult>\("handoff\.begin"/);
  assert.match(extensionSource, /const captured = await captureContext\(\);[\s\S]+client\.call<HandoffMutationResult>\("handoff\.begin"/);
  assert.match(extensionSource, /source_generation: captured[\s\S]+vscode-context-skipped/);
  assert.match(extensionSource, /restoreWorkspaceContext/);
  assert.match(extensionSource, /client\.call<EditorEventRecordResult>\("editor\.event\.record"/);
  assert.match(extensionSource, /statusBar\.command = "devrelay\.explainState"/);
});

test("extension wires edit guard event listeners", () => {
  assert.match(extensionSource, /onDidChangeTextDocument/);
  assert.match(extensionSource, /onDidSaveTextDocument/);
  assert.match(extensionSource, /onDidChangeActiveTextEditor/);
});

test("package exposes unsaved buffer safety settings", () => {
  const properties = packageJson.contributes.configuration.properties;
  assert.equal(properties["devrelay.captureEditorContext"].default, true);
  assert.equal(properties["devrelay.captureUnsavedBuffers"].default, false);
  assert.equal(properties["devrelay.includeUntitledUnsavedBuffers"].default, false);
});
