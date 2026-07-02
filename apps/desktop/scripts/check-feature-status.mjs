import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

// Invariant: every product surface that is designed but not wired to the
// local agent is declared exactly once in the FEATURES registry in
// src/app.js, and every registry entry renders the standard
// data-feature-pending "Not built yet" marker. When a feature is finished,
// remove the registry entry AND its markers together, following the
// procedure in apps/desktop/AGENTS.md. This check fails loudly when the two
// drift apart so leftover placeholders cannot survive a completed feature.

const here = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(here, "..");
const source = await readFile(resolve(appDir, "src/app.js"), "utf8");
const failures = [];

const startMarker = "/* FEATURE-REGISTRY-START";
const endMarker = "/* FEATURE-REGISTRY-END */";
const start = source.indexOf(startMarker);
const end = source.indexOf(endMarker);
let registry = null;

if (start === -1 || end === -1 || end < start) {
  failures.push("src/app.js is missing the FEATURE-REGISTRY-START/END block");
} else {
  const block = source.slice(start, end);
  const assignment = block.indexOf("const FEATURES =");
  if (assignment === -1) {
    failures.push("FEATURE registry block does not define `const FEATURES =`");
  } else {
    const literal = block.slice(assignment + "const FEATURES =".length).trim().replace(/;\s*$/, "");
    try {
      registry = vm.runInNewContext(`(${literal})`, {}, { timeout: 1000 });
    } catch (error) {
      failures.push(`FEATURES registry is not a plain object literal: ${error.message}`);
    }
  }
}

// Marker attributes are emitted dynamically by pendingChip/pendingPanel/
// pendingAction, so collect feature ids from those call sites plus any
// statically written data-feature/data-feature-pending literals.
const markerCalls = [...source.matchAll(/pending(?:Chip|Panel|Action)\(\s*"([^"]+)"/g)].map((match) => match[1]);
const staticAttrs = [...source.matchAll(/data-feature(?:-pending)?="([^"$]+)"/g)].map((match) => match[1]);
const pendingMarkers = [...markerCalls, ...staticAttrs];
const pendingActions = staticAttrs;

if (registry) {
  const ids = Object.keys(registry);
  if (ids.length === 0) {
    // An empty registry is legal: it means the complete product is fully
    // wired and no placeholder markers may remain anywhere.
    if (pendingMarkers.length > 0) {
      failures.push(
        "FEATURES registry is empty but pending markers remain in src/app.js; remove the leftover placeholders (see apps/desktop/AGENTS.md)"
      );
    }
  }
  for (const [id, entry] of Object.entries(registry)) {
    if (!entry || typeof entry !== "object") {
      failures.push(`FEATURES["${id}"] must be an object`);
      continue;
    }
    for (const key of ["title", "toast", "note", "area"]) {
      if (typeof entry[key] !== "string" || entry[key].trim() === "") {
        failures.push(`FEATURES["${id}"] is missing a non-empty "${key}" string`);
      }
    }
    if (typeof entry.toast === "string" && !/not wired to the agent yet$/.test(entry.toast)) {
      failures.push(`FEATURES["${id}"].toast must end with "not wired to the agent yet"`);
    }
    if (!pendingMarkers.includes(id)) {
      failures.push(
        `FEATURES["${id}"] never renders its "Not built yet" marker (pendingChip/pendingPanel with data-feature-pending="${id}")`
      );
    }
  }
  for (const id of new Set([...pendingMarkers, ...pendingActions])) {
    if (!registry[id]) {
      failures.push(
        `src/app.js references feature "${id}" but it is not in the FEATURES registry. ` +
          "If the feature is now wired to the agent, delete the leftover marker and its handler; " +
          "see apps/desktop/AGENTS.md for the removal procedure."
      );
    }
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"));
  console.error("\nSee apps/desktop/AGENTS.md (Not built yet markers) for how to keep the registry and UI in sync.");
  process.exit(1);
}

console.log(`feature status check passed (${registry ? Object.keys(registry).length : 0} pending features declared)`);
