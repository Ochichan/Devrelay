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
  ]);
});

test("contributed commands are registered by the extension", () => {
  const commands = packageJson.contributes.commands.map((command) => command.command);
  assert.deepEqual(commands, ["devrelay.refreshConnection", "devrelay.explainState"]);

  for (const command of commands) {
    assert.match(extensionSource, new RegExp(`registerCommand\\("${command}"`));
  }
});

test("extension surfaces local agent connection state", () => {
  assert.match(extensionSource, /createStatusBarItem/);
  assert.match(extensionSource, /client\.call<AgentHealthResult>\("agent\.health"\)/);
  assert.match(extensionSource, /statusBar\.command = "devrelay\.explainState"/);
});
