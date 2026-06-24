import { readFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Invariant: safety/ui_has_no_state_authority. The desktop frontend may call
// Tauri commands and consume agent events, but it must not read Git,
// filesystem, shell, or durable browser state as canonical product state.
const here = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(here, "..");
const appFiles = [resolve(appDir, "src/app.js"), resolve(appDir, "dist/app.js")];
const allowedCommands = new Set([
  "checkpoint_create",
  "diagnostics_export",
  "handoff_abort",
  "handoff_continue_here",
  "handoff_prepare",
  "open_project",
  "project_add",
  "project_status",
  "recover_open",
  "settings_update",
  "ui_bootstrap",
]);
const requiredAgentEvents = [
  "devrelay-agent-connected",
  "devrelay-agent-disconnected",
  "devrelay-agent-event",
  "devrelay-agent-gap",
];
const forbiddenStateSources = [
  {
    label: "Git command execution",
    pattern: /\bgit\s+(?:cat-file|checkout|diff|ls-files|rev-parse|reset|show|status)\b/i,
  },
  {
    label: "Git porcelain parsing",
    pattern: /--porcelain|porcelain\s+v2/i,
  },
  {
    label: "Tauri filesystem plugin",
    pattern: /@tauri-apps\/plugin-fs|__TAURI__\?*\.(?:fs)|\b(?:BaseDirectory|exists|readDir|readTextFile|stat)\b/,
  },
  {
    label: "Tauri shell/process plugin",
    pattern: /@tauri-apps\/plugin-shell|__TAURI__\?*\.(?:process|shell)|\bCommand\.(?:create|sidecar)\b/,
  },
  {
    label: "Node filesystem or process APIs",
    pattern: /node:fs|node:child_process|from\s+["'](?:fs|child_process)["']|require\(["'](?:fs|child_process)["']\)/,
  },
  {
    label: "Browser durable storage as canonical state",
    pattern: /\b(?:indexedDB|localStorage|sessionStorage)\b/,
  },
];

const failures = [];
const source = await readFile(resolve(appDir, "src/app.js"), "utf8");
const invokedCommands = [...source.matchAll(/\binvoke\(\s*["']([^"']+)["']/g)].map(
  (match) => match[1]
);

for (const command of invokedCommands) {
  if (!allowedCommands.has(command)) {
    failures.push(`src/app.js invokes non-agent-authorized command ${command}`);
  }
}

for (const eventName of requiredAgentEvents) {
  if (!source.includes(`listen("${eventName}"`)) {
    failures.push(`src/app.js does not listen for ${eventName}`);
  }
}

for (const file of appFiles) {
  const text = await readFile(file, "utf8");
  for (const { label, pattern } of forbiddenStateSources) {
    if (pattern.test(text)) {
      failures.push(`${relative(appDir, file)} uses forbidden UI state source: ${label}`);
    }
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log("safety/ui_has_no_state_authority passed");
