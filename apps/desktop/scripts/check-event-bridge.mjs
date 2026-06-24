import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(here, "..");
const source = await readFile(resolve(appDir, "src/app.js"), "utf8");

const handlers = new Map();
const app = {
  innerHTML: "",
  querySelectorAll: () => [],
  querySelector: () => null,
};
const nowSeconds = Math.floor(Date.now() / 1000);
const bootstrap = {
  runtime: {
    platform_key: "macos",
    architecture: "aarch64",
    agent_socket_exists: true,
    agent_socket_path: "/tmp/devrelay.sock",
    devrelay_home: "/tmp/devrelay-home",
  },
  agent: {
    connected: true,
    methods: ["handoff.begin"],
    errors: [],
  },
  settings: {
    device_id: "local-device",
    device_name: "Local device",
    fabric_name: "Local fabric",
    resource_profile: "adaptive",
    anchor_mode: "local",
    project_count: 0,
    mdns_enabled: false,
    editor_command: "code",
  },
  projects: [
    {
      project_id: "project-1",
      display_name: "Project One",
      local_path: "/tmp/project-one",
      manifest_path: "/tmp/project-one/devrelay.toml",
      workspaces: {
        main: {
          workspace_id: "session-1",
          state: "active",
          device_id: "local-device",
          local_path: "/tmp/project-one",
        },
      },
    },
  ],
  snapshots: [],
  leases: [
    {
      lease_id: "lease-1",
      project_id: "project-1",
      session_id: "session-1",
      state: "active",
      holder_device_id: "local-device",
      latest_snapshot_id: null,
      handoff_id: null,
    },
  ],
  handoffs: [],
  devices: [
    {
      device_id: "local-device",
      display_name: "Local device",
      platform_key: "darwin-arm64",
      architecture: "arm64",
      last_seen_unix_seconds: nowSeconds,
    },
    {
      device_id: "target-device",
      display_name: "Target device",
      platform_key: "linux-gnu-x86_64",
      architecture: "x86_64",
      last_seen_unix_seconds: nowSeconds,
    },
  ],
  runs: [],
  activity: [],
};
const cleanStatus = {
  ok: true,
  data: {
    status: {
      clean: true,
      counts: {
        staged: 0,
        unstaged: 0,
        untracked: 0,
        ignored: 0,
        unmerged: 0,
      },
    },
  },
};
const invoked = [];

const context = {
  document: {
    querySelector: (selector) => (selector === "#app" ? app : null),
  },
  window: {
    __TAURI__: {
      event: {
        listen: async (name, handler) => {
          handlers.set(name, handler);
          return () => {};
        },
      },
      core: {
        invoke: async (name) => {
          invoked.push(name);
          if (name === "project_status") return cleanStatus;
          return bootstrap;
        },
      },
    },
    setTimeout,
    clearTimeout,
  },
  crypto: {
    randomUUID: () => "toast-id",
  },
  Intl,
  Date,
  FormData: class FormData {},
};

vm.createContext(context);
vm.runInContext(source, context, { filename: "app.js" });
await new Promise((resolve) => setTimeout(resolve, 20));

assert.match(app.innerHTML, /Prepare handoff/, "ready target handoff action did not render");
assert.match(
  app.innerHTML,
  /target apply and verification remain pending/,
  "handoff panel did not keep verification pending"
);

for (const eventName of [
  "devrelay-tray-refresh",
  "devrelay-agent-connected",
  "devrelay-agent-event",
  "devrelay-agent-gap",
  "devrelay-agent-disconnected",
]) {
  assert.equal(typeof handlers.get(eventName), "function", `${eventName} listener was not installed`);
}

handlers.get("devrelay-agent-connected")({
  payload: {
    cursor: { after_sequence: 2 },
    replayed: 1,
    current_sequence: 3,
  },
});
handlers.get("devrelay-agent-event")({
  payload: {
    sequence: 4,
    type: "snapshot.local.created",
  },
});
handlers.get("devrelay-agent-gap")({
  payload: {
    expected_after: 4,
    actual_next: 7,
  },
});
handlers.get("devrelay-agent-disconnected")({
  payload: "socket closed",
});
handlers.get("devrelay-tray-refresh")({});
await new Promise((resolve) => setTimeout(resolve, 30));

assert(invoked.includes("ui_bootstrap"), "event bridge flow did not refresh bootstrap state");
assert.match(
  app.innerHTML,
  /Event stream reconnecting|Events reconnecting/,
  "event bridge reconnect state did not render"
);

console.log("event bridge check passed");
