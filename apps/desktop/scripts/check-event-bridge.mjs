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
    methods: [
      "apply.snapshot",
      "handoff.begin",
      "handoff.target.verify",
      "handoff.source.ready",
      "handoff.commit",
    ],
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
          if (name === "handoff_continue_here") {
            return {
              ok: true,
              message: "continuation verified",
              data: {
                handoff: {
                  handoff: {
                    handoff_id: "handoff-local",
                    state: "committed",
                  },
                  journal: [],
                },
              },
            };
          }
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
vm.runInContext(
  `
state.bootstrap.leases[0].state = "handoff-pending";
state.bootstrap.handoffs = [{
  record: {
    handoff_id: "handoff-1",
    project_id: "project-1",
    state: "target-prepare",
    target_device_id: "target-device",
    expires_at_unix_seconds: ${nowSeconds + 300},
  },
  journal: [],
}];
render();
`,
  context
);
assert.match(app.innerHTML, /Abort handoff/, "open handoff did not render abort action");
assert.doesNotMatch(app.innerHTML, /Prepare handoff/, "open handoff still rendered prepare action");
vm.runInContext(
  `
state.bootstrap.handoffs = [{
  record: {
    handoff_id: "handoff-local",
    project_id: "project-1",
    state: "target-prepare",
    target_device_id: "local-device",
    expires_at_unix_seconds: ${nowSeconds + 300},
  },
  journal: [],
}];
render();
`,
  context
);
assert.match(app.innerHTML, /Continue here/, "incoming handoff did not render continue action");
assert.match(
  app.innerHTML,
  /Ready to apply and verify/,
  "incoming handoff did not render target readiness"
);
await vm.runInContext(
  `
handleAction({
  dataset: {
    action: "handoff-continue-here",
    projectId: "project-1",
    handoffId: "handoff-local",
  },
});
`,
  context
);
assert(
  invoked.includes("handoff_continue_here"),
  "continue here action did not invoke Tauri command"
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
handlers.get("devrelay-agent-event")({
  payload: {
    sequence: 5,
    occurred_at_unix_millis: Date.now(),
    type: "handoff.state.changed",
    payload: {
      project_id: "project-1",
      handoff_id: "handoff-1",
      lease_id: "lease-1",
      previous_state: "target-prepare",
      state: "target-verified",
      source_device_id: "local-device",
      target_device_id: "target-device",
      expires_at_unix_seconds: nowSeconds + 300,
    },
  },
});
vm.runInContext('state.view = "activity"; render();', context);
assert.match(app.innerHTML, /Handoff events/, "activity view did not render handoff section");
assert.match(app.innerHTML, /Target Verified/, "handoff event state did not render");
assert.doesNotMatch(app.innerHTML, /lease-1/, "handoff event exposed lease id");
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
