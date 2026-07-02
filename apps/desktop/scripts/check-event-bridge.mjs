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
const documentHandlers = new Map();
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
  environments: [
    {
      project_id: "project-1",
      workspace_id: "session-1",
      state: "shell-ready",
      attempt: 2,
      failure: null,
      updated_at_unix_seconds: nowSeconds - 15,
      persisted: true,
    },
  ],
  devices: [
    {
      device_id: "local-device",
      display_name: "Local device",
      platform_key: "darwin-arm64",
      architecture: "arm64",
      capabilities_json: JSON.stringify({
        anchor: true,
        local_snapshots: true,
        filesystem_events: true,
        fsmonitor: true,
      }),
      last_seen_unix_seconds: nowSeconds,
      resource_summary: {
        cpu: "8 cores, idle",
        memory: "16 GB total",
        disk: "420 GB free / 1 TB total",
        power: "AC, low power off",
        cache_warmth: "Checkpoint metadata ready",
      },
    },
    {
      device_id: "target-device",
      display_name: "Target device",
      platform_key: "linux-gnu-x86_64",
      architecture: "x86_64",
      capabilities_json: JSON.stringify({
        anchor: true,
        local_snapshots: true,
        filesystem_events: true,
      }),
      last_seen_unix_seconds: nowSeconds,
      resource_summary: {
        cpu: "12 cores, idle",
        memory: "32 GB total",
        disk: "180 GB free / 512 GB total",
        power: "AC",
        cache_warmth: "Warm cache",
      },
    },
    {
      device_id: "offline-device",
      display_name: "Offline device",
      platform_key: "linux-gnu-aarch64",
      architecture: "aarch64",
      capabilities_json: JSON.stringify({
        local_snapshots: true,
      }),
      last_seen_unix_seconds: nowSeconds - 3600,
    },
  ],
  runs: [
    {
      task_run_id: "run-queued-1",
      project_id: "project-1",
      session_id: "session-1",
      state: "queued",
      command: "npm test",
      metadata: {
        scheduler_reason: "Target has warm cache",
        target_device_id: "target-device",
        artifact_count: 0,
      },
      created_at_unix_seconds: nowSeconds - 50,
      updated_at_unix_seconds: nowSeconds - 40,
    },
    {
      task_run_id: "run-running-1",
      project_id: "project-1",
      session_id: "session-1",
      state: "running",
      command: "cargo test",
      metadata: {
        scheduler_explanation: "Local writer is idle",
        target_device_id: "local-device",
        artifacts: ["target/log.txt"],
      },
      created_at_unix_seconds: nowSeconds - 120,
      updated_at_unix_seconds: nowSeconds - 20,
    },
    {
      task_run_id: "run-failed-1",
      project_id: "project-2",
      session_id: "session-2",
      state: "failed",
      command: "cargo clippy",
      metadata: {
        scheduler_reason: "Target selected before failure",
        artifact_count: 2,
      },
      created_at_unix_seconds: nowSeconds - 240,
      updated_at_unix_seconds: nowSeconds - 180,
    },
    {
      task_run_id: "run-done-1",
      project_id: "project-1",
      session_id: "session-1",
      state: "succeeded",
      command: "npm run build",
      metadata: {
        scheduler_reason: "Completed on local device",
        artifact_count: 1,
      },
      created_at_unix_seconds: nowSeconds - 360,
      updated_at_unix_seconds: nowSeconds - 300,
    },
  ],
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
      head_oid: "status-head",
      branch: "main",
      upstream: "origin/main",
      ahead: 2,
      behind: 0,
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
    querySelectorAll: () => [],
    addEventListener: (name, handler) => {
      documentHandlers.set(name, handler);
    },
    activeElement: null,
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

assert.match(app.innerHTML, /Review handoff/, "ready target handoff review action did not render");
assert.match(
  app.innerHTML,
  /target apply and verification remain pending/,
  "handoff panel did not keep verification pending"
);
assert.match(app.innerHTML, /Environment hydration/, "continue view did not render environment hydration");
assert.match(app.innerHTML, /Shell Ready/, "continue view did not render hydration state");
assert.match(app.innerHTML, /Checkpoint metadata ready/, "continue view did not render warmth summary");
assert.match(app.innerHTML, /Run elsewhere/, "continue view did not render run elsewhere placeholder");
assert.match(app.innerHTML, /aria-label="Review handoff to Target device"/, "handoff review action lacked target label");
assert.match(app.innerHTML, /Not built yet/, "continue view did not render the not-built marker copy");
assert.match(
  app.innerHTML,
  /data-feature-pending="continue.run-elsewhere"/,
  "run elsewhere did not carry the standard pending marker"
);
await vm.runInContext(
  `
handleAction({
  dataset: {
    action: "feature-pending",
    feature: "continue.run-elsewhere",
    projectId: "project-1",
  },
});
`,
  context
);
assert.match(app.innerHTML, /Run elsewhere is not wired to the agent yet/, "run placeholder did not warn");
await vm.runInContext(
  `
handleAction({
  dataset: {
    action: "handoff-dialog",
    projectId: "project-1",
    targetDeviceId: "target-device",
  },
});
`,
  context
);
assert.match(app.innerHTML, /Handoff review/, "handoff dialog did not render");
assert.match(app.innerHTML, /Source device/, "handoff dialog did not render source device");
assert.match(app.innerHTML, /Target device/, "handoff dialog did not render target device");
assert.match(app.innerHTML, /Project\/session/, "handoff dialog did not render project session");
assert.match(app.innerHTML, /tabindex="-1"/, "handoff dialog did not expose a focus target");
assert.match(app.innerHTML, /Checkpoint age/, "handoff dialog did not render checkpoint age");
assert.match(app.innerHTML, /0 staged/, "handoff dialog did not render staged count");
assert.match(app.innerHTML, /0 modified, 0 new/, "handoff dialog did not render modified and untracked counts");
assert.match(app.innerHTML, /2 commits not pushed/, "handoff dialog did not render unpushed commits");
assert.match(app.innerHTML, /Environment readiness/, "handoff dialog did not render environment readiness");
assert.match(app.innerHTML, /Editor context readiness/, "handoff dialog did not render editor context readiness");
assert.match(app.innerHTML, /Target safety/, "handoff dialog did not render target safety");
assert.match(app.innerHTML, /Separate target work/, "handoff dialog did not render target preservation copy");
assert.match(app.innerHTML, /No overwrite/, "handoff dialog did not render no-overwrite target copy");
assert.match(
  app.innerHTML,
  /preserve it separately, open incoming work in a new folder, or cancel safely/,
  "handoff dialog did not render non-jargon target options"
);
assert.doesNotMatch(
  app.innerHTML,
  /snapshot-and-fork|new-workspace|lease|epoch|OID|CAS|canonical latest/i,
  "handoff dialog exposed internal dirty-target terminology"
);
assert.match(app.innerHTML, /Saving state/, "handoff dialog did not render saving state progress");
assert.match(app.innerHTML, /Preparing device/, "handoff dialog did not render preparing device progress");
assert.match(app.innerHTML, /Moving control/, "handoff dialog did not render moving control progress");
assert.match(app.innerHTML, /Start handoff/, "handoff dialog did not render start action");
vm.runInContext('state.handoffDialog.phase = "failure"; state.handoffDialog.message = "handoff failed"; render();', context);
assert.match(app.innerHTML, /Handoff failed/, "handoff dialog did not render failure state");
vm.runInContext('state.handoffDialog.phase = "success"; state.handoffDialog.message = "Target preparation has started"; render();', context);
assert.match(app.innerHTML, /Handoff started/, "handoff dialog did not render success state");
assert.equal(typeof documentHandlers.get("keydown"), "function", "global keydown listener was not installed");
let escapePrevented = false;
documentHandlers.get("keydown")({
  key: "Escape",
  preventDefault: () => {
    escapePrevented = true;
  },
});
assert.equal(escapePrevented, true, "Escape did not prevent default while closing dialog");
assert.equal(vm.runInContext("state.handoffDialog", context), null, "Escape did not close handoff dialog");
vm.runInContext('state.view = "runs"; render();', context);
assert.match(app.innerHTML, /Recent runs/, "runs view did not render recent runs");
assert.match(app.innerHTML, /Queued runs/, "runs view did not render queued runs");
assert.match(app.innerHTML, /Running runs/, "runs view did not render running runs");
assert.match(app.innerHTML, /Failed runs/, "runs view did not render failed runs");
assert.match(app.innerHTML, /Scheduler explanation/, "runs view did not render scheduler explanation");
assert.match(app.innerHTML, /Target has warm cache/, "runs view did not render scheduler reason");
assert.match(app.innerHTML, /Run task/, "runs view did not render run task placeholder");
assert.match(app.innerHTML, /Cancel/, "runs view did not render cancel placeholder");
assert.match(app.innerHTML, /Artifacts/, "runs view did not render artifact placeholder");
assert.match(app.innerHTML, /Target device/, "runs view did not render target device name");
assert.match(app.innerHTML, /aria-label="Cancel run run-queued-1"/, "run cancel action lacked run label");
assert.match(
  app.innerHTML,
  /data-feature-pending="runs.start"/,
  "run task did not carry the standard pending marker"
);
assert.match(
  app.innerHTML,
  /data-feature-pending="runs.cancel"/,
  "run cancel did not carry the standard pending marker"
);
await vm.runInContext(
  `
handleAction({
  dataset: {
    action: "feature-pending",
    feature: "runs.start",
  },
});
`,
  context
);
assert.match(app.innerHTML, /Run task is not wired to the agent yet/, "run task placeholder did not warn");
vm.runInContext('state.view = "settings"; render();', context);
assert.match(app.innerHTML, /Background behavior/, "settings view did not render background behavior settings");
assert.match(app.innerHTML, /Storage and cache/, "settings view did not render storage settings");
assert.match(app.innerHTML, /Network/, "settings view did not render network settings");
assert.match(app.innerHTML, /Security/, "settings view did not render security settings");
assert.match(app.innerHTML, /Editor context/, "settings view did not render editor context settings");
assert.match(app.innerHTML, /Advanced diagnostics/, "settings view did not render advanced diagnostics settings");
assert.match(app.innerHTML, /Save settings/, "settings view did not render save action");
assert.match(app.innerHTML, /Checkpoint cache/, "settings view did not render cache state");
assert.match(
  app.innerHTML,
  /data-feature-pending="settings.retention"/,
  "retention controls did not carry the standard pending marker"
);
assert.equal(
  vm.runInContext(
    'validateSettingsInput({ get: (name) => name === "resource_profile" ? "adaptive" : name === "editor_command" ? "code" : null })',
    context
  ),
  null,
  "settings validation rejected a valid payload"
);
assert.match(
  vm.runInContext(
    'validateSettingsInput({ get: (name) => name === "resource_profile" ? "adaptive" : name === "editor_command" ? "  " : null })',
    context
  ),
  /Editor command is required/,
  "settings validation did not reject blank editor command"
);
vm.runInContext('state.view = "devices"; render();', context);
assert.match(app.innerHTML, /3 known identities - 2 online/, "devices view did not render device counts");
assert.match(app.innerHTML, /Pair device/, "devices view did not render pair placeholder");
assert.match(app.innerHTML, /Revoke/, "devices view did not render revoke placeholder");
assert.match(app.innerHTML, /aria-label="Revoke Target device"/, "device revoke action lacked device label");
assert.match(app.innerHTML, /Online/, "devices view did not render online state");
assert.match(app.innerHTML, /Offline/, "devices view did not render offline state");
assert.match(app.innerHTML, /macOS/, "devices view did not render OS family");
assert.match(app.innerHTML, /arm64/, "devices view did not render architecture");
assert.match(app.innerHTML, /Local Snapshots/, "devices view did not render capabilities");
assert.match(app.innerHTML, /8 cores, idle/, "devices view did not render CPU summary");
assert.match(app.innerHTML, /16 GB total/, "devices view did not render memory summary");
assert.match(app.innerHTML, /420 GB free/, "devices view did not render disk summary");
assert.match(app.innerHTML, /AC, low power off/, "devices view did not render battery or AC state");
assert.match(app.innerHTML, /Warm cache/, "devices view did not render cache warmth");
assert.match(
  app.innerHTML,
  /data-feature-pending="devices.pair"/,
  "pair device did not carry the standard pending marker"
);
assert.match(
  app.innerHTML,
  /data-feature-pending="devices.revoke"/,
  "device revoke did not carry the standard pending marker"
);
await vm.runInContext(
  `
handleAction({
  dataset: {
    action: "feature-pending",
    feature: "devices.pair",
  },
});
`,
  context
);
assert.match(app.innerHTML, /Pair device is not wired to the agent yet/, "pair placeholder did not warn");
vm.runInContext('state.view = "projects"; render();', context);
assert.match(app.innerHTML, /Active session/, "projects view did not render session column");
assert.match(app.innerHTML, /Writer/, "projects view did not render writer column");
assert.match(app.innerHTML, /Checkpoint/, "projects view did not render checkpoint column");
assert.match(app.innerHTML, /1\/2 ready/, "projects view did not render target availability");
assert.match(app.innerHTML, /Needs attention \(1\)/, "projects view did not render attention group");
assert.match(app.innerHTML, /Ready \(1\)/, "projects view did not render ready group");
assert.match(app.innerHTML, /Add project/, "projects view did not render add project entry point");
assert.match(app.innerHTML, /Open recovery/, "projects view did not render recovery entry point");
assert.match(app.innerHTML, /Project path/, "projects view did not render project add path field");
assert.match(app.innerHTML, /Recovery path/, "projects view did not render recovery target field");
assert.match(app.innerHTML, /Filter projects/, "projects view did not render filter");
assert.match(app.innerHTML, /Details/, "projects view did not render project detail action");
assert.match(app.innerHTML, /Recovery/, "projects view did not render row recovery action");
assert.match(app.innerHTML, /aria-label="Refresh status for Project One"/, "project status action lacked project label");
await vm.runInContext(
  `
handleAction({
  dataset: {
    action: "project-recovery",
    projectId: "project-1",
  },
});
`,
  context
);
assert.equal(
  vm.runInContext("state.recoveryProjectId", context),
  "project-1",
  "project recovery action did not select project"
);
assert.equal(
  vm.runInContext("state.recoverySnapshotId", context),
  "s1_projectonecheckpoint",
  "project recovery action did not select latest snapshot"
);
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
  "devrelay-tray-notice",
  "devrelay-tray-open-runs",
  "devrelay-agent-connected",
  "devrelay-agent-event",
  "devrelay-agent-gap",
  "devrelay-agent-disconnected",
]) {
  assert.equal(typeof handlers.get(eventName), "function", `${eventName} listener was not installed`);
}

handlers.get("devrelay-tray-notice")({
  payload: {
    message: "Tray action completed",
    kind: "good",
  },
});
assert.match(app.innerHTML, /Tray action completed/, "tray notice did not render toast");
assert.match(app.innerHTML, /role="status" aria-live="polite"/, "tray notice did not render live region");
handlers.get("devrelay-tray-open-runs")({
  payload: {
    project_id: "project-1",
    target_device_id: "target-device",
    target_label: "Target device",
  },
});
assert.equal(vm.runInContext("state.view", context), "runs", "tray run shortcut did not open runs view");
assert.equal(
  vm.runInContext("state.selectedProjectId", context),
  "project-1",
  "tray run shortcut did not preserve project context"
);
assert.match(
  app.innerHTML,
  /Run elsewhere for Target device is not wired to the agent yet/,
  "tray run shortcut did not warn"
);
vm.runInContext('state.view = "continue"; render();', context);

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

documentHandlers.get("keydown")({
  key: "k",
  metaKey: true,
  preventDefault: () => {},
});
assert.match(app.innerHTML, /data-palette-input/, "command menu did not open on the keyboard shortcut");
assert.match(app.innerHTML, /Not built yet/, "command menu did not mark not-built commands");
documentHandlers.get("keydown")({
  key: "Escape",
  preventDefault: () => {},
});
assert.equal(vm.runInContext("state.palette", context), null, "Escape did not close the command menu");

console.log("event bridge check passed");
