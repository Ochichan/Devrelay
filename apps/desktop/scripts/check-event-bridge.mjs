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
    {
      project_id: "project-2",
      display_name: "Project Two",
      local_path: "/tmp/project-two",
      manifest_path: "/tmp/project-two/devrelay.toml",
      workspaces: {
        main: {
          workspace_id: "session-2",
          state: "inactive",
          device_id: "target-device",
          local_path: "/tmp/project-two",
        },
      },
    },
  ],
  snapshots: [
    {
      snapshot_id: "s1_projectonecheckpoint",
      project_id: "project-1",
      session_id: "session-1",
      sequence_number: 1,
      label: "desktop",
      created_at_unix_seconds: nowSeconds,
      metadata: {
        state_hash: "state-project-1",
      },
    },
  ],
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
  activity: [
    {
      audit_id: 1,
      project_id: "project-1",
      type: "checkpoint.create",
      outcome: "succeeded",
      summary: "Checkpoint created",
      detail: {
        internal_oid: "oid-secret",
      },
      created_at_unix_seconds: nowSeconds,
    },
  ],
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
vm.runInContext('state.view = "projects"; render();', context);
assert.match(app.innerHTML, /Active session/, "projects view did not render session column");
assert.match(app.innerHTML, /Writer/, "projects view did not render writer column");
assert.match(app.innerHTML, /Checkpoint/, "projects view did not render checkpoint column");
assert.match(app.innerHTML, /1\/1 ready/, "projects view did not render target availability");
assert.match(app.innerHTML, /Needs attention \(1\)/, "projects view did not render attention group");
assert.match(app.innerHTML, /Ready \(1\)/, "projects view did not render ready group");
assert.match(app.innerHTML, /Filter projects/, "projects view did not render filter");
assert.match(app.innerHTML, /Details/, "projects view did not render project detail action");
vm.runInContext('state.projectFilter = "two"; render();', context);
assert.match(app.innerHTML, /Project Two/, "project filter did not keep matching project");
assert.doesNotMatch(
  app.innerHTML,
  /data-project-id="project-1"/,
  "project filter did not hide non-matching table action"
);
vm.runInContext('state.projectFilter = ""; state.view = "continue"; render();', context);
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
    occurred_at_unix_millis: Date.now(),
    type: "snapshot.local.created",
    payload: {
      project_id: "project-1",
      snapshot_id: "s1_livecheckpoint000000000",
      snapshot_sequence_number: 2,
      label: "desktop",
      state_hash: "state-secret",
      created_at_unix_seconds: nowSeconds,
    },
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
handlers.get("devrelay-agent-event")({
  payload: {
    sequence: 6,
    occurred_at_unix_millis: Date.now(),
    type: "snapshot.apply.verified",
    payload: {
      project_id: "project-1",
      snapshot_id: "s1_livecheckpoint000000000",
      target_workspace_id: "session-1",
      verification: {
        head_oid: "oid-secret",
        index_tree_oid: "oid-secret",
        work_tree_oid: "oid-secret",
        state_hash: "state-secret",
      },
    },
  },
});
handlers.get("devrelay-agent-event")({
  payload: {
    sequence: 7,
    occurred_at_unix_millis: Date.now(),
    type: "security.blocked",
    payload: {
      code: "DR-SECURITY-BLOCKED",
      title: "Secret excluded",
      detail: "Blocked a private key file",
      action: "Review excluded files",
      project_id: "project-1",
      safe_actions: ["Remove the secret"],
    },
  },
});
handlers.get("devrelay-agent-event")({
  payload: {
    sequence: 8,
    occurred_at_unix_millis: Date.now(),
    type: "quota.warning",
    payload: {
      quota: "snapshot-store",
      scope: "project",
      used: 90,
      limit: 100,
      unit: "mb",
      project_id: "project-1",
      detail: "Snapshot store is near its configured limit",
    },
  },
});
vm.runInContext('state.view = "activity"; render();', context);
assert.match(app.innerHTML, /Activity filters/, "activity view did not render filters");
assert.match(app.innerHTML, /Audit events/, "activity view did not render audit section");
assert.match(app.innerHTML, /Checkpoint events/, "activity view did not render checkpoint section");
assert.match(app.innerHTML, /Handoff events/, "activity view did not render handoff section");
assert.match(app.innerHTML, /Security blocks/, "activity view did not render security section");
assert.match(app.innerHTML, /Quota warnings/, "activity view did not render quota section");
assert.match(app.innerHTML, /Diagnostics/, "activity view did not render diagnostics action");
assert.match(app.innerHTML, /Checkpoint Created/, "checkpoint event summary did not render");
assert.match(app.innerHTML, /Target Apply Verified/, "apply verified summary did not render");
assert.match(app.innerHTML, /Target Verified/, "handoff event state did not render");
assert.match(app.innerHTML, /Secret excluded/, "security block did not render");
assert.match(app.innerHTML, /snapshot-store/, "quota warning did not render");
assert.doesNotMatch(app.innerHTML, /lease-1/, "handoff event exposed lease id");
assert.doesNotMatch(app.innerHTML, /oid-secret/, "activity view exposed internal OID detail");
assert.doesNotMatch(app.innerHTML, /state-secret/, "activity view exposed internal state hash");
vm.runInContext('state.activityFilter = "checkpoint"; render();', context);
assert.match(app.innerHTML, /Checkpoint events/, "checkpoint filter hid checkpoint section");
assert.doesNotMatch(app.innerHTML, /Handoff events/, "checkpoint filter did not hide handoff section");
vm.runInContext('state.activityFilter = "all"; render();', context);
handlers.get("devrelay-agent-gap")({
  payload: {
    expected_after: 8,
    actual_next: 11,
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
