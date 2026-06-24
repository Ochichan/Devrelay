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
    "onCommand:devrelay.captureContext",
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
    "devrelay.captureContext",
    "devrelay.captureUnsavedBuffers",
    "devrelay.restoreUnsavedBuffers",
    "devrelay.openDashboard",
  ]);

  for (const command of commands) {
    assert.match(extensionSource, new RegExp(`registerCommand\\("${command}"`));
  }
});

test("extension surfaces local agent connection state", () => {
  assert.match(extensionSource, /createStatusBarItem/);
  assert.match(extensionSource, /client\.call<AgentHealthResult>\("agent\.health"\)/);
  assert.match(extensionSource, /client\.call<EditorContextUpdateResult>\(\s*"editor\.context\.update"/);
  assert.match(extensionSource, /statusBar\.command = "devrelay\.explainState"/);
});

test("package exposes unsaved buffer safety settings", () => {
  const properties = packageJson.contributes.configuration.properties;
  assert.equal(properties["devrelay.captureUnsavedBuffers"].default, false);
  assert.equal(properties["devrelay.includeUntitledUnsavedBuffers"].default, false);
});
