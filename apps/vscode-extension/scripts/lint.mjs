import { readdir, readFile } from "node:fs/promises";
import { join, relative, resolve } from "node:path";

const root = resolve(new URL("..", import.meta.url).pathname);
const checkedRoots = [resolve(root, "src"), resolve(root, "test")];
const failures = [];

async function files(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const found = [];
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      found.push(...(await files(path)));
    } else if (/\.(ts|mjs)$/.test(entry.name)) {
      found.push(path);
    }
  }
  return found;
}

for (const checkedRoot of checkedRoots) {
  for (const file of await files(checkedRoot)) {
    const text = await readFile(file, "utf8");
    const label = relative(root, file);
    if (/\t/.test(text)) {
      failures.push(`${label} contains a tab character`);
    }
    if (/[ \t]$/m.test(text)) {
      failures.push(`${label} contains trailing whitespace`);
    }
    if (text.includes("console.log")) {
      failures.push(`${label} uses console.log instead of the extension output channel`);
    }
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log("VS Code extension lint passed");
