import { readFile, readdir } from "node:fs/promises";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { dirname } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(here, "..");
const roots = [resolve(appDir, "src"), resolve(appDir, "dist")];
const banned = [
  "payments-api",
  "Mac Studio",
  "Linux Workstation",
  "Windows / WSL",
  "web-console",
  "mobile-shell",
  "legacy-admin",
  "data-pipeline",
  "feature/refund",
  "Ryzen",
  "RTX",
  "M4 Max",
  "scheduler score",
  "Nix cache 97",
  "9d3fa82",
  "41.2s",
];

async function files(root) {
  const out = [];
  async function walk(dir) {
    const entries = await readdir(dir, { withFileTypes: true });
    for (const entry of entries) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        await walk(path);
      } else {
        out.push(path);
      }
    }
  }
  await walk(root);
  return out;
}

const failures = [];
for (const root of roots) {
  for (const file of await files(root)) {
    const text = await readFile(file, "utf8");
    for (const marker of banned) {
      if (text.includes(marker)) {
        failures.push(`${relative(appDir, file)} contains ${marker}`);
      }
    }
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log("UI source check passed");
