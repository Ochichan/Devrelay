/*
 * DevRelay desktop frontend.
 *
 * Contract (enforced by scripts/check-ui-state-authority.mjs and
 * scripts/check-event-bridge.mjs):
 *   - The local agent is the only state authority. This file may call the
 *     whitelisted Tauri commands and fold agent events into `state`; it must
 *     never read Git, the filesystem, shells, or durable browser storage.
 *   - The UI never declares that work moved. Handoff progress is rendered
 *     from agent handoff records and events only, and target apply plus
 *     verification stay visibly pending until the agent confirms them.
 *   - Rendered copy never exposes internal wire terminology.
 *
 * Complete-product shell with explicit not-built markers:
 *   The layout below is the final product surface. Features the agent cannot
 *   back yet are declared once in the FEATURES registry and must render the
 *   standard "Not built yet" marker. scripts/check-feature-status.mjs keeps
 *   the registry and the markers in sync. Removal procedure for finished
 *   features lives in apps/desktop/AGENTS.md.
 */

const app = document.querySelector("#app");

/* FEATURE-REGISTRY-START
 * Registry of product surfaces that are designed but not wired to the local
 * agent yet. Every entry must be rendered with pendingChip(id) and routed
 * through data-action="feature-pending". When the agent gains the backing
 * RPC, wire the real action, delete the entry here, delete its markers, and
 * update scripts/check-event-bridge.mjs plus apps/desktop/AGENTS.md.
 */
const FEATURES = {
  "fabric.switch": {
    title: "Fabric switching",
    area: "shell",
    toast: "Fabric switching is not wired to the agent yet",
    note: "The agent manages a single local fabric today.",
  },
  "continue.run-elsewhere": {
    title: "Run elsewhere",
    area: "continue",
    toast: "Run elsewhere is not wired to the agent yet",
    note: "Remote run dispatch has no agent RPC yet.",
  },
  "continue.route-telemetry": {
    title: "Route telemetry",
    area: "continue",
    toast: "Route telemetry is not wired to the agent yet",
    note: "The agent does not report link timing or transfer volume yet.",
  },
  "insights.protection-trend": {
    title: "Protection trend",
    area: "continue",
    toast: "Protection trend is not wired to the agent yet",
    note: "Local metrics aggregation is not exposed to the desktop yet.",
  },
  "scheduler.controls": {
    title: "Scheduler controls",
    area: "continue",
    toast: "Scheduler controls are not wired to the agent yet",
    note: "The agent explains past target choices but takes no scheduling commands.",
  },
  "projects.pick-folder": {
    title: "Folder picker",
    area: "projects",
    toast: "Folder picking is not wired to the agent yet",
    note: "The UI cannot browse the filesystem; type the project path instead.",
  },
  "devices.pair": {
    title: "Pair device",
    area: "devices",
    toast: "Pair device is not wired to the agent yet",
    note: "Pairing runs through the CLI until the desktop pairing flow lands.",
  },
  "devices.revoke": {
    title: "Device revoke",
    area: "devices",
    toast: "Device revoke is not wired to the agent yet",
    note: "Revocation runs through the CLI until the desktop flow lands.",
  },
  "devices.policy": {
    title: "Resource policy",
    area: "devices",
    toast: "Resource policy is not wired to the agent yet",
    note: "Per-device resource policy editing has no agent RPC yet.",
  },
  "runs.start": {
    title: "Run task",
    area: "runs",
    toast: "Run task is not wired to the agent yet",
    note: "The agent has no task-start RPC yet.",
  },
  "runs.cancel": {
    title: "Run cancel",
    area: "runs",
    toast: "Run cancel is not wired to the agent yet",
    note: "The agent has no run-cancel RPC yet.",
  },
  "runs.artifacts": {
    title: "Run artifacts",
    area: "runs",
    toast: "Run artifacts are not wired to the agent yet",
    note: "Artifact retrieval is not exposed to the desktop yet.",
  },
  "runs.cache-history": {
    title: "Cache history",
    area: "runs",
    toast: "Cache history is not wired to the agent yet",
    note: "Result-cache history is not exposed to the desktop yet.",
  },
  "settings.retention": {
    title: "Retention controls",
    area: "settings",
    toast: "Retention controls are not wired to the agent yet",
    note: "Retention runs on agent defaults; advanced controls come later.",
  },
  "settings.standby-target": {
    title: "Standby target",
    area: "settings",
    toast: "Standby target is not wired to the agent yet",
    note: "A default continue target cannot be pinned through the agent yet.",
  },
  "editor.context-restore": {
    title: "Editor context restore",
    area: "settings",
    toast: "Editor context restore is not wired to the agent yet",
    note: "Editor context restore is driven from the VS Code extension today.",
  },
};
/* FEATURE-REGISTRY-END */

const views = [
  ["continue", "Continue", "play", "Workspace"],
  ["projects", "Projects", "folder", "Workspace"],
  ["devices", "Devices", "monitor", "Workspace"],
  ["runs", "Runs", "terminal", "Workspace"],
  ["activity", "Activity", "pulse", "System"],
  ["settings", "Settings", "settings", "System"],
];

const resourceProfiles = ["adaptive", "instant", "eco", "custom", "balanced", "performance"];

const state = {
  view: "continue",
  theme: "auto",
  loading: true,
  operation: null,
  selectedProjectId: null,
  projectFilter: "",
  recoveryProjectId: null,
  recoverySnapshotId: null,
  activityFilter: "all",
  handoffDialog: null,
  palette: null,
  bootstrap: null,
  projectStatus: new Map(),
  runtimeError: null,
  eventBridge: {
    connected: false,
    everConnected: false,
    refreshing: false,
    stale: false,
    lastConnectedAt: null,
    lastDisconnectedAt: null,
    lastEvent: null,
    lastGap: null,
    lastError: null,
    subscription: null,
    events: [],
  },
  toasts: [],
  pendingFocusSelector: null,
};

/* ------------------------------------------------------------ runtime */

function invoke(name, params) {
  const runtimeInvoke = window.__TAURI__?.core?.invoke || window.__TAURI__?.tauri?.invoke;
  if (!runtimeInvoke) {
    return Promise.reject(new Error("Desktop runtime is not available"));
  }
  return runtimeInvoke(name, params);
}

function listen(name, handler) {
  const runtimeListen = window.__TAURI__?.event?.listen;
  if (!runtimeListen) return Promise.resolve(() => {});
  return runtimeListen(name, handler);
}

/* -------------------------------------------------------------- utils */

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function icon(name, extra = "") {
  return `<svg class="icon${extra ? ` ${extra}` : ""}" aria-hidden="true"><use href="#i-${name}"></use></svg>`;
}

function selectorValue(value) {
  return String(value ?? "")
    .replaceAll("\\", "\\\\")
    .replaceAll('"', '\\"');
}

function attrSelector(name, value) {
  if (value === null || value === undefined || value === "") return "";
  return `[${name}="${selectorValue(value)}"]`;
}

function actionFocusSelector(button) {
  if (!button?.dataset?.action) return null;
  return [
    attrSelector("data-action", button.dataset.action),
    attrSelector("data-project-id", button.dataset.projectId),
    attrSelector("data-target-device-id", button.dataset.targetDeviceId),
    attrSelector("data-handoff-id", button.dataset.handoffId),
  ].join("");
}

function queueFocus(selector) {
  state.pendingFocusSelector = selector || null;
}

function applyPendingFocus() {
  const selector = state.pendingFocusSelector;
  if (!selector) return;
  state.pendingFocusSelector = null;
  window.setTimeout(() => {
    app.querySelector(selector)?.focus?.();
  }, 0);
}

function formatAge(seconds) {
  if (!seconds) return "never";
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
  if (delta < 10) return "just now";
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
}

function formatUntil(seconds) {
  if (!seconds) return "unknown";
  const delta = Math.floor(seconds - Date.now() / 1000);
  if (delta <= 0) return "expired";
  if (delta < 60) return `${delta}s`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h`;
  return `${Math.floor(delta / 86400)}d`;
}

function formatClock(milliseconds) {
  if (!milliseconds) return "never";
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(milliseconds));
}

function shortId(value) {
  const text = String(value ?? "");
  if (text.length <= 12) return text || "-";
  return `${text.slice(0, 8)}...${text.slice(-4)}`;
}

function titleize(value) {
  return String(value ?? "")
    .replaceAll("-", " ")
    .replaceAll("_", " ")
    .replace(/\b\w/g, (match) => match.toUpperCase());
}

function plural(count, word) {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

function parseJsonObject(value) {
  if (!value || typeof value !== "string") return {};
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

let toastCounter = 0;

function toast(message, kind = "good") {
  const id = crypto.randomUUID?.() ?? `toast-${(toastCounter += 1)}`;
  state.toasts.push({ id, message, kind });
  render();
  window.setTimeout(() => {
    state.toasts = state.toasts.filter((entry) => entry.id !== id);
    render();
  }, 5000);
}

/* --------------------------------------------- not-built-yet markers */

function featureEntry(featureId) {
  return FEATURES[featureId] ?? null;
}

function pendingChip(featureId) {
  const entry = featureEntry(featureId);
  if (!entry) return "";
  return `<span class="pending-chip" data-feature-pending="${escapeHtml(featureId)}" title="${escapeHtml(
    `${entry.title}: designed for the complete product, not backed by the local agent yet. ${entry.note}`
  )}">${icon("cone")}Not built yet</span>`;
}

function pendingAction(featureId, label, extraClass = "", extraAria = "") {
  const entry = featureEntry(featureId);
  if (!entry) return "";
  const aria = extraAria || label;
  return `<button type="button" class="button ${extraClass}" data-action="feature-pending" data-feature="${escapeHtml(
    featureId
  )}" aria-label="${escapeHtml(aria)}" title="${escapeHtml(entry.note)}">${escapeHtml(label)}${pendingChip(featureId)}</button>`;
}

function pendingPanel(featureId, body) {
  const entry = featureEntry(featureId);
  if (!entry) return "";
  return `<div class="pending-panel" data-feature-pending="${escapeHtml(featureId)}">${icon("cone")}<div><strong>${escapeHtml(
    entry.title
  )}</strong> is not built yet. ${escapeHtml(body || entry.note)}</div></div>`;
}

/* ---------------------------------------------------- bootstrap reads */

function methods() {
  return new Set(state.bootstrap?.agent?.methods ?? []);
}

function projects() {
  return state.bootstrap?.projects ?? [];
}

function devices() {
  return state.bootstrap?.devices ?? [];
}

function snapshots() {
  return state.bootstrap?.snapshots ?? [];
}

function leases() {
  return state.bootstrap?.leases ?? [];
}

function handoffs() {
  return state.bootstrap?.handoffs ?? [];
}

function environments() {
  return state.bootstrap?.environments ?? [];
}

function runs() {
  return state.bootstrap?.runs ?? [];
}

function activity() {
  return state.bootstrap?.activity ?? [];
}

function settings() {
  return state.bootstrap?.settings ?? null;
}

function liveEvents() {
  return state.eventBridge.events;
}

function localDeviceId() {
  return settings()?.device_id ?? null;
}

function deviceById(deviceId) {
  return devices().find((device) => device.device_id === deviceId) ?? null;
}

function deviceName(deviceId) {
  if (!deviceId) return "Unknown device";
  return deviceById(deviceId)?.display_name ?? shortId(deviceId);
}

function projectById(projectId) {
  return projects().find((project) => project.project_id === projectId) ?? null;
}

function projectName(projectId) {
  if (!projectId) return "Unknown project";
  return projectById(projectId)?.display_name ?? shortId(projectId);
}

function selectedProject() {
  const list = projects();
  if (list.length === 0) return null;
  return list.find((project) => project.project_id === state.selectedProjectId) ?? list[0];
}

/* --------------------------------------------------- derived insight */

const ONLINE_WINDOW_SECONDS = 300;

function deviceOnline(device) {
  if (!device?.last_seen_unix_seconds) return false;
  return Date.now() / 1000 - device.last_seen_unix_seconds < ONLINE_WINDOW_SECONDS;
}

function deviceOsFamily(device) {
  const key = String(device?.platform_key ?? "").toLowerCase();
  if (key.includes("darwin") || key.includes("macos")) return "macOS";
  if (key.includes("wsl")) return "WSL";
  if (key.includes("windows") || key.includes("msvc")) return "Windows";
  if (key.includes("linux")) return "Linux";
  return titleize(device?.platform_key ?? "Unknown");
}

function deviceGlyphTone(device) {
  const family = deviceOsFamily(device);
  if (family === "macOS") return "mac";
  if (family === "Linux") return "linux";
  if (family === "Windows" || family === "WSL") return "win";
  return "generic";
}

function deviceGlyphIcon(device) {
  const family = deviceOsFamily(device);
  if (family === "macOS") return "laptop";
  if (family === "Linux") return "server";
  return "monitor";
}

function deviceCapabilities(device) {
  return Object.entries(parseJsonObject(device?.capabilities_json))
    .filter(([, enabled]) => Boolean(enabled))
    .map(([name]) => titleize(name));
}

function remoteDevices() {
  const local = localDeviceId();
  return devices().filter((device) => device.device_id !== local);
}

function readyTargets() {
  return remoteDevices().filter((device) => deviceOnline(device));
}

function targetAvailability() {
  const remote = remoteDevices();
  return { ready: readyTargets().length, total: remote.length };
}

function projectSnapshots(projectId) {
  return snapshots()
    .filter((snapshot) => snapshot.project_id === projectId)
    .sort((a, b) => (b.created_at_unix_seconds ?? 0) - (a.created_at_unix_seconds ?? 0));
}

function latestSnapshot(projectId) {
  return projectSnapshots(projectId)[0] ?? null;
}

function projectWriter(projectId) {
  const active = leases().find(
    (entry) => entry.project_id === projectId && (entry.state === "active" || entry.state === "handoff-pending")
  );
  if (!active) return null;
  return {
    deviceId: active.holder_device_id ?? null,
    state: active.state,
    local: active.holder_device_id === localDeviceId(),
  };
}

function openHandoffs(projectId) {
  return handoffs().filter((entry) => {
    const record = entry.record ?? entry;
    if (projectId && record.project_id !== projectId) return false;
    return record.state === "target-prepare" || record.state === "target-verified" || record.state === "source-ready";
  });
}

function incomingHandoff(entry) {
  const record = entry.record ?? entry;
  return record.target_device_id === localDeviceId();
}

function projectEnvironments(projectId) {
  return environments().filter((entry) => entry.project_id === projectId);
}

function environmentReadiness(projectId) {
  const entry = projectEnvironments(projectId)[0];
  if (!entry) return "No hydration record yet";
  return titleize(entry.state);
}

function localCacheWarmth() {
  const local = deviceById(localDeviceId());
  return local?.resource_summary?.cache_warmth ?? null;
}

function projectRuns(projectId) {
  return runs()
    .filter((run) => run.project_id === projectId)
    .sort((a, b) => (b.updated_at_unix_seconds ?? 0) - (a.updated_at_unix_seconds ?? 0));
}

function runsByState(stateName) {
  return runs()
    .filter((run) => run.state === stateName)
    .sort((a, b) => (b.updated_at_unix_seconds ?? 0) - (a.updated_at_unix_seconds ?? 0));
}

function schedulerNote(run) {
  const metadata = run?.metadata && typeof run.metadata === "object" ? run.metadata : {};
  return metadata.scheduler_explanation ?? metadata.scheduler_reason ?? null;
}

function runTargetDeviceId(run) {
  const metadata = run?.metadata && typeof run.metadata === "object" ? run.metadata : {};
  return metadata.target_device_id ?? null;
}

function runArtifactCount(run) {
  const metadata = run?.metadata && typeof run.metadata === "object" ? run.metadata : {};
  if (Array.isArray(metadata.artifacts)) return metadata.artifacts.length;
  if (typeof metadata.artifact_count === "number") return metadata.artifact_count;
  return 0;
}

function projectNeedsAttention(project) {
  const projectId = project.project_id;
  if (openHandoffs(projectId).length > 0) return true;
  const workspaceStates = Object.values(project.workspaces ?? {}).map((workspace) => workspace.state);
  if (workspaceStates.some((value) => value === "inactive" || value === "stale")) return true;
  const failedEnvironment = projectEnvironments(projectId).some((entry) => entry.failure);
  if (failedEnvironment) return true;
  return false;
}

function checkpointEvents() {
  return liveEvents().filter((event) => String(event.type ?? "").startsWith("snapshot."));
}

function handoffEvents() {
  return liveEvents().filter((event) => event.type === "handoff.state.changed");
}

function securityEvents() {
  return liveEvents().filter((event) => event.type === "security.blocked");
}

function quotaEvents() {
  return liveEvents().filter((event) => event.type === "quota.warning");
}

function snapshotEventSummary(event) {
  if (event.type === "snapshot.local.created") return "Checkpoint Created";
  if (event.type === "snapshot.apply.started") return "Target Apply Started";
  if (event.type === "snapshot.apply.verified") return "Target Apply Verified";
  return titleize(String(event.type ?? "").replaceAll(".", " "));
}

function eventBridgeStatus() {
  const bridge = state.eventBridge;
  if (bridge.stale) return { tone: "warn", label: "Event gap" };
  if (bridge.connected && bridge.refreshing) return { tone: "sync", label: "Events syncing" };
  if (bridge.connected) return { tone: "live", label: "Events live" };
  if (bridge.everConnected) return { tone: "off", label: "Events reconnecting" };
  return { tone: "sync", label: "Events connecting" };
}

function agentHealth() {
  const bridge = state.eventBridge;
  const agent = state.bootstrap?.agent;
  if (!agent?.connected) return { tone: "off", label: "Agent offline" };
  if (bridge.stale || !bridge.connected) return { tone: "warn", label: eventBridgeStatus().label };
  if ((agent.errors ?? []).length > 0) return { tone: "warn", label: "Agent degraded" };
  return { tone: "online", label: "All systems healthy" };
}

/* ---------------------------------------------------------- shell UI */

function navCount(viewId) {
  if (viewId === "projects") return projects().length;
  if (viewId === "devices") return devices().length;
  if (viewId === "runs") return runsByState("queued").length + runsByState("running").length;
  return null;
}

function renderNav() {
  const groups = new Map();
  for (const [id, label, iconName, group] of views) {
    if (!groups.has(group)) groups.set(group, []);
    const count = navCount(id);
    groups.get(group).push(`
      <button class="nav-item${state.view === id ? " active" : ""}" data-view="${id}" aria-current="${
        state.view === id ? "page" : "false"
      }">
        ${icon(iconName)}
        <span class="nav-label">${label}</span>
        ${count !== null && count > 0 ? `<span class="nav-count">${count}</span>` : ""}
      </button>`);
  }
  return [...groups.entries()]
    .map(
      ([group, items]) => `
      <div class="nav-group">
        <div class="nav-group-label">${group}</div>
        ${items.join("")}
      </div>`
    )
    .join("");
}

function renderSidebar() {
  const config = settings();
  const fabricName = config?.fabric_name ?? "Local fabric";
  const deviceCount = devices().length;
  const project = selectedProject();
  const writer = project ? projectWriter(project.project_id) : null;
  const snapshot = project ? latestSnapshot(project.project_id) : null;
  const agent = agentHealth();
  return `
    <aside class="sidebar">
      <div class="brand">
        <div class="brand-mark">${icon("relay")}</div>
        <div class="brand-text">
          <strong>DevRelay</strong>
          <span>Personal Dev Fabric</span>
        </div>
      </div>
      <button class="fabric-btn" data-action="feature-pending" data-feature="fabric.switch" title="${escapeHtml(
        FEATURES["fabric.switch"].note
      )}">
        <span class="presence-dot ${agent.tone === "online" ? "online" : agent.tone === "warn" ? "warn" : "off"}"></span>
        <span class="fabric-name">
          <strong>${escapeHtml(fabricName)}</strong>
          <span>${plural(deviceCount, "device")} · ${escapeHtml(config?.anchor_mode ? `${titleize(config.anchor_mode)} anchor` : "anchor unknown")}</span>
        </span>
        ${pendingChip("fabric.switch")}
      </button>
      <nav class="nav" aria-label="Primary">${renderNav()}</nav>
      <div class="sidebar-spacer"></div>
      ${
        project
          ? `
      <button class="mini-card" data-project="${escapeHtml(project.project_id)}" aria-label="Open ${escapeHtml(
        project.display_name
      )} in Continue" style="cursor: pointer; font: inherit; color: inherit;">
        <span class="mini-top">
          ${writer?.local ? '<span class="live-dot" aria-hidden="true"></span>' : ""}
          <strong>${escapeHtml(project.display_name)}</strong>
        </span>
        <span class="mini-meta">
          <span>${writer ? (writer.local ? "Writing here" : `Active on ${escapeHtml(deviceName(writer.deviceId))}`) : "No active writer"}</span>
          <span>${snapshot ? escapeHtml(formatAge(snapshot.created_at_unix_seconds)) : "no checkpoint"}</span>
        </span>
      </button>`
          : ""
      }
      <div class="anchor-card">
        <div class="anchor-avatar">${icon("shield")}</div>
        <div class="anchor-text">
          <strong>Local agent</strong>
          <span>${escapeHtml(agent.label)}</span>
        </div>
        <span class="presence-dot ${agent.tone === "online" ? "online" : agent.tone === "warn" ? "warn" : "off"}"></span>
      </div>
    </aside>`;
}

function viewLabel(viewId) {
  return views.find(([id]) => id === viewId)?.[1] ?? "Continue";
}

function renderTopbar() {
  const config = settings();
  const agent = agentHealth();
  const bridge = eventBridgeStatus();
  const alerts = securityEvents().length + quotaEvents().length;
  return `
    <header class="topbar">
      <div class="crumbs">
        <span>${escapeHtml(config?.fabric_name ?? "Local fabric")}</span>
        <span class="crumb-sep">/</span>
        <strong>${viewLabel(state.view)}</strong>
      </div>
      <div class="topbar-spacer"></div>
      <button class="command-trigger" data-action="palette-open" aria-label="Open command menu">
        ${icon("search")}
        <span>Search or jump to</span>
        <kbd>&#8984;K</kbd>
      </button>
      <div class="top-actions">
        <div class="health-pill" title="${escapeHtml(bridge.label)}">
          <span class="presence-dot ${agent.tone === "online" ? "online" : agent.tone === "warn" ? "warn" : "off"}"></span>
          <span>${escapeHtml(agent.label)}</span>
        </div>
        <span class="chip ${bridge.tone === "live" ? "good" : bridge.tone === "warn" ? "warn" : bridge.tone === "off" ? "bad" : "info"}">${escapeHtml(
          bridge.label
        )}</span>
        <button class="icon-btn" data-action="open-activity" aria-label="Open activity${alerts > 0 ? ` (${alerts} notices)` : ""}">
          ${icon("bell")}
          ${alerts > 0 ? '<span class="badge-dot" aria-hidden="true"></span>' : ""}
        </button>
        <button class="icon-btn" data-action="toggle-theme" aria-label="Switch color theme (current: ${escapeHtml(state.theme)})">
          ${icon(resolvedTheme() === "dark" ? "moon" : "sun")}
        </button>
        <button class="icon-btn" data-action="refresh" aria-label="Refresh state">
          ${icon("history")}
        </button>
      </div>
    </header>`;
}

function resolvedTheme() {
  if (state.theme === "dark" || state.theme === "light") return state.theme;
  const prefersLight = window.matchMedia?.("(prefers-color-scheme: light)")?.matches ?? false;
  return prefersLight ? "light" : "dark";
}

/* ------------------------------------------------------ continue view */

function statusFor(projectId) {
  return state.projectStatus.get(projectId) ?? null;
}

function statusCounts(projectId) {
  const entry = statusFor(projectId);
  const status = entry?.data?.status ?? null;
  if (!status) return null;
  const counts = status.counts ?? {};
  return {
    branch: status.branch ?? null,
    head: status.head_oid ?? null,
    ahead: status.ahead ?? 0,
    clean: Boolean(status.clean),
    staged: counts.staged ?? 0,
    modified: counts.unstaged ?? 0,
    untracked: counts.untracked ?? 0,
  };
}

function changesLine(projectId) {
  const counts = statusCounts(projectId);
  if (!counts) return "Working tree summary is loading from the agent.";
  const parts = [
    `${counts.modified} modified`,
    `${counts.staged} staged`,
    `${counts.untracked} untracked`,
  ];
  if (counts.ahead > 0) parts.push(`${counts.ahead} not pushed`);
  return parts.join(" · ");
}

function writerChip(projectId) {
  const writer = projectWriter(projectId);
  if (!writer) return '<span class="chip"><span class="chip-dot"></span>No active writer</span>';
  if (writer.state === "handoff-pending") {
    return '<span class="chip warn"><span class="chip-dot"></span>Handoff pending</span>';
  }
  if (writer.local) return '<span class="chip good"><span class="chip-dot"></span>Writing on this device</span>';
  return `<span class="chip violet"><span class="chip-dot"></span>Active on ${escapeHtml(deviceName(writer.deviceId))}</span>`;
}

function checkpointChip(projectId) {
  const snapshot = latestSnapshot(projectId);
  if (!snapshot) return '<span class="chip warn"><span class="chip-dot"></span>No checkpoint yet</span>';
  return `<span class="chip info"><span class="chip-dot"></span>Protected ${escapeHtml(formatAge(snapshot.created_at_unix_seconds))}</span>`;
}

function environmentChip(projectId) {
  const entry = projectEnvironments(projectId)[0];
  if (!entry) return "";
  const tone = entry.failure ? "bad" : entry.state === "shell-ready" || entry.state === "ready" ? "good" : "info";
  return `<span class="chip ${tone}"><span class="chip-dot"></span>Environment ${escapeHtml(titleize(entry.state))}</span>`;
}

function bestTarget() {
  return readyTargets()[0] ?? null;
}

function renderTargetRow(project, device) {
  const online = deviceOnline(device);
  const name = escapeHtml(device.display_name);
  return `
    <div class="device-row">
      <div class="device-glyph ${deviceGlyphTone(device)}">${icon(deviceGlyphIcon(device))}</div>
      <div class="device-row-main">
        <strong>${name}</strong>
        <span>${escapeHtml(deviceOsFamily(device))} · ${escapeHtml(device.architecture ?? "unknown")} · ${
          online ? "Ready" : "Offline"
        }</span>
      </div>
      ${
        online
          ? `<button class="button small primary" data-action="handoff-dialog" data-project-id="${escapeHtml(
              project.project_id
            )}" data-target-device-id="${escapeHtml(device.device_id)}" aria-label="Review handoff to ${name}">Review handoff</button>`
          : '<span class="chip"><span class="chip-dot"></span>Offline</span>'
      }
    </div>`;
}

function handoffStepIndex(handoffState) {
  if (handoffState === "target-prepare") return 1;
  if (handoffState === "target-verified") return 2;
  if (handoffState === "source-ready") return 2;
  return 0;
}

function renderHandoffSteps(activeIndex, doneAll = false) {
  const labels = ["Saving state", "Preparing device", "Moving control"];
  return `<ol class="steps">${labels
    .map((label, index) => {
      const doneStep = doneAll || index < activeIndex;
      const active = !doneAll && index === activeIndex;
      return `<li class="step${doneStep ? " done" : ""}${active ? " active" : ""}"><span class="step-dot"></span>${label}</li>`;
    })
    .join("")}</ol>`;
}

function renderOpenHandoff(entry) {
  const record = entry.record ?? entry;
  const project = escapeHtml(projectName(record.project_id));
  const incoming = incomingHandoff(entry);
  const stepIndex = handoffStepIndex(record.state);
  const expiry = record.expires_at_unix_seconds ? `window closes in ${formatUntil(record.expires_at_unix_seconds)}` : "";
  if (incoming) {
    const agentReady = methods().has("apply.snapshot") && methods().has("handoff.target.verify");
    return `
      <div class="device-row" data-handoff-row="${escapeHtml(record.handoff_id)}">
        <div class="device-glyph linux">${icon("download")}</div>
        <div class="device-row-main">
          <strong>Incoming: ${project}</strong>
          <span>${escapeHtml(titleize(record.state))}${expiry ? ` · ${escapeHtml(expiry)}` : ""}</span>
        </div>
        <span class="chip ${agentReady ? "good" : "warn"}">${agentReady ? "Ready to apply and verify" : "Agent update required"}</span>
        <button class="button small primary" data-action="handoff-continue-here" data-project-id="${escapeHtml(
          record.project_id
        )}" data-handoff-id="${escapeHtml(record.handoff_id)}" aria-label="Continue ${project} here">Continue here</button>
      </div>`;
  }
  return `
    <div class="device-row" data-handoff-row="${escapeHtml(record.handoff_id)}">
      <div class="device-glyph mac">${icon("relay")}</div>
      <div class="device-row-main">
        <strong>${project} &rarr; ${escapeHtml(deviceName(record.target_device_id))}</strong>
        <span>${escapeHtml(titleize(record.state))}${expiry ? ` · ${escapeHtml(expiry)}` : ""}</span>
      </div>
      <button class="button small danger" data-action="handoff-abort" data-project-id="${escapeHtml(
        record.project_id
      )}" data-handoff-id="${escapeHtml(record.handoff_id)}" aria-label="Abort handoff for ${project}">Abort handoff</button>
    </div>
    ${renderHandoffSteps(stepIndex)}`;
}

function renderRouteCard(project) {
  const local = deviceById(localDeviceId());
  const target = bestTarget();
  const snapshot = latestSnapshot(project.project_id);
  const counts = statusCounts(project.project_id);
  const availability = targetAvailability();
  return `
    <aside class="hero-route">
      <div class="route-head">
        <span>Continuation route</span>
        ${pendingChip("continue.route-telemetry")}
      </div>
      <div class="route-map">
        <div class="route-node active">
          <div class="device-glyph ${local ? deviceGlyphTone(local) : "generic"}">${icon(local ? deviceGlyphIcon(local) : "monitor")}</div>
          <div class="route-node-name">
            <strong>${escapeHtml(local?.display_name ?? "This device")}</strong>
            <span>Writer · ${escapeHtml(local ? deviceOsFamily(local) : "unknown")}</span>
          </div>
        </div>
        <div class="route-line" aria-hidden="true"></div>
        <div class="route-node">
          <div class="device-glyph ${target ? deviceGlyphTone(target) : "generic"}">${icon(target ? deviceGlyphIcon(target) : "monitor")}</div>
          <div class="route-node-name">
            <strong>${escapeHtml(target?.display_name ?? "No ready target")}</strong>
            <span>${target ? `Ready · ${escapeHtml(deviceOsFamily(target))}` : "Pair or wake a second device"}</span>
          </div>
        </div>
      </div>
      <div class="route-stats">
        <div class="metric">
          <span class="metric-value">${snapshot ? escapeHtml(formatAge(snapshot.created_at_unix_seconds)) : "none"}</span>
          <span class="metric-label">Checkpoint</span>
        </div>
        <div class="metric">
          <span class="metric-value">${counts ? counts.modified + counts.staged + counts.untracked : "&ndash;"}</span>
          <span class="metric-label">Tracked changes</span>
        </div>
        <div class="metric">
          <span class="metric-value">${availability.ready}/${availability.total || 0}</span>
          <span class="metric-label">Targets ready</span>
        </div>
      </div>
    </aside>`;
}

function renderContinue() {
  const project = selectedProject();
  const config = settings();
  if (!project) {
    return `
      <div class="page-head">
        <div>
          <div class="eyebrow"><span class="eyebrow-dot"></span>${escapeHtml(config?.device_name ?? "This device")}</div>
          <h1 class="page-title">Continue your work</h1>
          <p class="page-sub">Register a project to start protecting and moving work between your devices.</p>
        </div>
        <div class="page-actions">
          <button class="button primary" data-view="projects">Add project</button>
        </div>
      </div>
      <div class="card"><div class="card-body"><div class="empty">${icon("folder")}<strong>No projects yet</strong><span>Add a Git project from the Projects screen to begin.</span></div></div></div>`;
  }
  const projectId = project.project_id;
  const counts = statusCounts(projectId);
  const snapshot = latestSnapshot(projectId);
  const target = bestTarget();
  const open = openHandoffs(projectId);
  const warmth = localCacheWarmth();
  const environmentRows = projectEnvironments(projectId);
  const schedulerRows = projectRuns(projectId)
    .map((run) => ({ run, note: schedulerNote(run) }))
    .filter((entry) => entry.note)
    .slice(0, 3);
  return `
    <div class="page-head">
      <div>
        <div class="eyebrow"><span class="eyebrow-dot"></span>${escapeHtml(config?.device_name ?? "This device")} · ${escapeHtml(
          titleize(state.bootstrap?.runtime?.platform_key ?? "unknown")
        )}</div>
        <h1 class="page-title">Continue your work</h1>
      </div>
      <div class="page-actions">
        ${pendingAction("continue.run-elsewhere", "Run elsewhere")}
        <button class="button" data-action="checkpoint" data-project-id="${escapeHtml(projectId)}" aria-label="Checkpoint ${escapeHtml(
          project.display_name
        )} now">${icon("shield")}Checkpoint now</button>
      </div>
    </div>
    <section class="hero">
      <div class="hero-main">
        <div class="hero-chips">
          ${writerChip(projectId)}
          ${checkpointChip(projectId)}
          ${environmentChip(projectId)}
        </div>
        <div>
          <div class="hero-kicker">Continuing work</div>
          <h2 class="hero-title">${escapeHtml(project.display_name)}</h2>
        </div>
        <div class="hero-branch">${icon("branch")}<code>${escapeHtml(counts?.branch ?? "branch loading")}${
          counts?.head ? ` · ${escapeHtml(shortId(counts.head))}` : ""
        }</code></div>
        <p class="hero-desc">${escapeHtml(changesLine(projectId))}</p>
        <div class="hero-actions">
          ${
            target
              ? `<button class="button primary" data-action="handoff-dialog" data-project-id="${escapeHtml(
                  projectId
                )}" data-target-device-id="${escapeHtml(target.device_id)}" aria-label="Review handoff to ${escapeHtml(
                  target.display_name
                )}">${icon("arrow-right")}Review handoff</button>`
              : '<span class="chip warn"><span class="chip-dot"></span>No ready target device</span>'
          }
          <button class="button" data-action="open-project" data-project-id="${escapeHtml(projectId)}" aria-label="Open ${escapeHtml(
            project.display_name
          )} in the editor">${icon("terminal")}Open in editor</button>
          <button class="button ghost" data-action="project-status" data-project-id="${escapeHtml(
            projectId
          )}" aria-label="Refresh status for ${escapeHtml(project.display_name)}">Refresh status</button>
        </div>
        <div class="hero-foot">${icon("lock")}Current work is checkpointed before any move; nothing on a target is overwritten.</div>
      </div>
      ${renderRouteCard(project)}
    </section>
    <div class="bento">
      <div class="bento-col">
        <section class="card">
          <div class="card-head">
            <div>
              <h3 class="card-title">${icon("relay")}Handoff</h3>
              <p class="card-sub">Movement of editing control, driven by the agent.</p>
            </div>
          </div>
          <div class="card-body">
            ${
              open.length > 0
                ? open.map((entry) => renderOpenHandoff(entry)).join("")
                : '<div class="empty">' + icon("relay") + "<strong>No handoff in flight</strong><span>Review a target below to move this work.</span></div>"
            }
            <p class="quiet">After a handoff starts, target apply and verification remain pending until the agent confirms them on the target device.</p>
          </div>
        </section>
        <section class="card">
          <div class="card-head">
            <div>
              <h3 class="card-title">${icon("monitor")}Continue on</h3>
              <p class="card-sub">${targetAvailability().ready}/${targetAvailability().total || 0} ready targets</p>
            </div>
          </div>
          <div class="card-body">
            ${
              remoteDevices().length > 0
                ? remoteDevices()
                    .map((device) => renderTargetRow(project, device))
                    .join("")
                : '<div class="empty">' + icon("monitor") + "<strong>No paired devices</strong><span>Pair a second device to continue work elsewhere.</span></div>"
            }
          </div>
        </section>
      </div>
      <div class="bento-col">
        <section class="card">
          <div class="card-head">
            <div>
              <h3 class="card-title">${icon("zap")}Environment hydration</h3>
              <p class="card-sub">Target readiness reported by the agent.</p>
            </div>
          </div>
          <div class="card-body">
            ${
              environmentRows.length > 0
                ? `<dl class="kv-list">${environmentRows
                    .map(
                      (entry) => `
                    <div class="kv">
                      <dt>${escapeHtml(entry.workspace_id ?? "workspace")}</dt>
                      <dd>${escapeHtml(titleize(entry.state))}${entry.attempt ? ` · attempt ${entry.attempt}` : ""}${
                        entry.updated_at_unix_seconds ? ` · ${escapeHtml(formatAge(entry.updated_at_unix_seconds))}` : ""
                      }</dd>
                    </div>
                    ${entry.failure ? `<div class="banner bad">${icon("x")}<div class="banner-body"><strong>Hydration failed</strong><span>${escapeHtml(entry.failure)}</span></div></div>` : ""}`
                    )
                    .join("")}</dl>`
                : '<p class="muted">No hydration record for this project yet.</p>'
            }
            ${warmth ? `<div class="kv"><dt class="muted">Cache warmth</dt><dd>${escapeHtml(warmth)}</dd></div>` : ""}
          </div>
        </section>
        <section class="card">
          <div class="card-head">
            <div>
              <h3 class="card-title">${icon("shield")}Protection</h3>
              <p class="card-sub">${plural(projectSnapshots(projectId).length, "checkpoint")} stored for this project.</p>
            </div>
            ${pendingChip("insights.protection-trend")}
          </div>
          <div class="card-body">
            <dl class="kv-list">
              <div class="kv"><dt>Latest checkpoint</dt><dd>${
                snapshot ? `${escapeHtml(formatAge(snapshot.created_at_unix_seconds))} · #${snapshot.sequence_number}` : "none yet"
              }</dd></div>
              <div class="kv"><dt>Working tree</dt><dd>${counts ? (counts.clean ? "Clean" : "Uncommitted changes present") : "loading"}</dd></div>
            </dl>
            ${pendingPanel("insights.protection-trend", "Checkpoint frequency and protection trend charts arrive with local metrics.")}
          </div>
        </section>
        <section class="card">
          <div class="card-head">
            <div>
              <h3 class="card-title">${icon("sliders")}Scheduler insight</h3>
              <p class="card-sub">Why the agent chose recent run targets.</p>
            </div>
            ${pendingChip("scheduler.controls")}
          </div>
          <div class="card-body">
            ${
              schedulerRows.length > 0
                ? `<ul class="check-list">${schedulerRows
                    .map(
                      (entry) => `<li class="check-item">${icon("check")}<span>${escapeHtml(entry.note)}<span class="check-note">${escapeHtml(
                        entry.run.command ?? "task"
                      )} · ${escapeHtml(titleize(entry.run.state))}</span></span></li>`
                    )
                    .join("")}</ul>`
                : '<p class="muted">No scheduled runs for this project yet.</p>'
            }
          </div>
        </section>
      </div>
    </div>`;
}

/* ------------------------------------------------------ handoff dialog */

function renderHandoffDialog() {
  const dialog = state.handoffDialog;
  if (!dialog) return "";
  const project = projectById(dialog.projectId);
  const targetDevice = deviceById(dialog.targetDeviceId);
  const local = deviceById(localDeviceId());
  const counts = statusCounts(dialog.projectId);
  const snapshot = latestSnapshot(dialog.projectId);
  const workspaceId = Object.values(project?.workspaces ?? {})[0]?.workspace_id ?? null;
  const phase = dialog.phase ?? "review";
  const title =
    phase === "failure" ? "Handoff failed" : phase === "success" ? "Handoff started" : "Handoff review";
  const running = phase === "running";
  const body =
    phase === "failure" || phase === "success"
      ? `<div class="banner ${phase === "failure" ? "bad" : ""}">${icon(phase === "failure" ? "x" : "check")}<div class="banner-body"><strong>${escapeHtml(
          title
        )}</strong><span>${escapeHtml(dialog.message ?? "")}</span></div></div>
        ${phase === "success" ? renderHandoffSteps(1) : ""}`
      : `
      <dl class="summary-grid">
        <div class="summary-row"><dt class="summary-term">Source device</dt><dd class="summary-val">${escapeHtml(
          local?.display_name ?? "This device"
        )}</dd></div>
        <div class="summary-row"><dt class="summary-term">Target device</dt><dd class="summary-val">${escapeHtml(
          targetDevice?.display_name ?? "Unknown device"
        )}</dd></div>
        <div class="summary-row"><dt class="summary-term">Project/session</dt><dd class="summary-val">${escapeHtml(
          project?.display_name ?? "Unknown project"
        )}${workspaceId ? ` · ${escapeHtml(shortId(workspaceId))}` : ""}</dd></div>
        <div class="summary-row"><dt class="summary-term">Checkpoint age</dt><dd class="summary-val">${
          snapshot ? escapeHtml(formatAge(snapshot.created_at_unix_seconds)) : "none yet"
        }</dd></div>
      </dl>
      <div class="divider"></div>
      <p class="section-label">Work being moved</p>
      <dl class="summary-grid">
        <div class="summary-row"><dt class="summary-term">Staged</dt><dd class="summary-val">${counts ? counts.staged : 0} staged</dd></div>
        <div class="summary-row"><dt class="summary-term">Working tree</dt><dd class="summary-val">${counts ? counts.modified : 0} modified, ${
          counts ? counts.untracked : 0
        } new</dd></div>
        <div class="summary-row"><dt class="summary-term">Unpushed</dt><dd class="summary-val">${counts ? counts.ahead : 0} commits not pushed</dd></div>
        <div class="summary-row"><dt class="summary-term">Environment readiness</dt><dd class="summary-val">${escapeHtml(
          environmentReadiness(dialog.projectId)
        )}</dd></div>
        <div class="summary-row"><dt class="summary-term">Editor context readiness</dt><dd class="summary-val">Captured by the editor extension when available</dd></div>
      </dl>
      <div class="divider"></div>
      <p class="section-label">Target safety</p>
      <ul class="check-list">
        <li class="check-item">${icon("shield")}<span><strong>Separate target work</strong><span class="check-note">If the target has its own work, you can preserve it separately, open incoming work in a new folder, or cancel safely.</span></span></li>
        <li class="check-item">${icon("lock")}<span><strong>No overwrite</strong><span class="check-note">Nothing on the target is replaced before the agent verifies the applied checkpoint.</span></span></li>
      </ul>
      <div class="divider"></div>
      <p class="section-label">Progress</p>
      ${renderHandoffSteps(running ? 1 : 0)}
      <p class="quiet">Keep this device idle for this project until the move completes; target apply and verification remain pending until the agent confirms them.</p>`;
  return `
    <div class="overlay" data-overlay="handoff">
      <div class="dialog" data-handoff-dialog role="dialog" aria-modal="true" aria-label="${escapeHtml(title)}" tabindex="-1">
        <div class="dialog-head">
          <div>
            <h2 class="dialog-title">${escapeHtml(title)}</h2>
            <p class="dialog-sub">${escapeHtml(project?.display_name ?? "")} &rarr; ${escapeHtml(
              targetDevice?.display_name ?? ""
            )}</p>
          </div>
          <button class="icon-btn" data-action="handoff-dialog-close" aria-label="Close handoff dialog">${icon("x")}</button>
        </div>
        <div class="dialog-body">${body}</div>
        <div class="dialog-foot">
          ${
            phase === "review" || phase === "running"
              ? `<button class="button ghost" data-action="handoff-dialog-close">Cancel safely</button>
                 <button class="button primary" data-action="handoff-confirm" data-project-id="${escapeHtml(
                   dialog.projectId
                 )}" data-target-device-id="${escapeHtml(dialog.targetDeviceId)}" ${running ? "disabled" : ""}>${
                   running ? "Starting" : "Start handoff"
                 }</button>`
              : '<button class="button primary" data-action="handoff-dialog-close">Done</button>'
          }
        </div>
      </div>
    </div>`;
}

/* ------------------------------------------------------- projects view */

function filteredProjects() {
  const filter = state.projectFilter.trim().toLowerCase();
  if (!filter) return projects();
  return projects().filter((project) => {
    const haystack = `${project.display_name ?? ""} ${project.project_id ?? ""} ${project.local_path ?? ""}`.toLowerCase();
    return haystack.includes(filter);
  });
}

function renderProjectRow(project) {
  const projectId = project.project_id;
  const name = escapeHtml(project.display_name);
  const workspace = Object.values(project.workspaces ?? {})[0] ?? null;
  const writer = projectWriter(projectId);
  const snapshot = latestSnapshot(projectId);
  const availability = targetAvailability();
  return `
    <tr>
      <td>
        <div class="project-cell">
          <div class="project-cell-glyph">${icon("folder")}</div>
          <div class="project-cell-name">
            <strong>${name}</strong>
            <span>${escapeHtml(project.local_path ?? "")}</span>
          </div>
        </div>
      </td>
      <td>${workspace ? `${escapeHtml(shortId(workspace.workspace_id))} · ${escapeHtml(titleize(workspace.state))}` : "none"}</td>
      <td>${writer ? escapeHtml(deviceName(writer.deviceId)) : "None"}</td>
      <td>${snapshot ? escapeHtml(formatAge(snapshot.created_at_unix_seconds)) : "none"}</td>
      <td>${availability.ready}/${availability.total || 0} ready</td>
      <td>
        <div class="row-actions">
          <button class="button small" data-project="${escapeHtml(projectId)}" aria-label="Open details for ${name}">Details</button>
          <button class="button small ghost" data-action="project-status" data-project-id="${escapeHtml(
            projectId
          )}" aria-label="Refresh status for ${name}">${icon("history")}</button>
          <button class="button small ghost" data-action="project-recovery" data-project-id="${escapeHtml(
            projectId
          )}" aria-label="Open recovery for ${name}">Recovery</button>
        </div>
      </td>
    </tr>`;
}

function renderProjectGroup(label, list) {
  if (list.length === 0) return "";
  return `
    <p class="group-label">${escapeHtml(label)} (${list.length})</p>
    <div class="table-scroll">
      <table class="data-table">
        <thead>
          <tr>
            <th>Project</th>
            <th>Active session</th>
            <th>Writer</th>
            <th>Checkpoint</th>
            <th>Availability</th>
            <th></th>
          </tr>
        </thead>
        <tbody>${list.map((project) => renderProjectRow(project)).join("")}</tbody>
      </table>
    </div>`;
}

function renderRecoveryForm() {
  const list = projects();
  const recoveryProject = list.find((project) => project.project_id === state.recoveryProjectId) ?? list[0] ?? null;
  const recoverySnapshots = recoveryProject ? projectSnapshots(recoveryProject.project_id) : [];
  const chosenSnapshot =
    recoverySnapshots.find((snapshot) => snapshot.snapshot_id === state.recoverySnapshotId) ?? recoverySnapshots[0] ?? null;
  return `
    <section class="card" id="recovery-card">
      <div class="card-head">
        <div>
          <h3 class="card-title">${icon("history")}Open recovery</h3>
          <p class="card-sub">Materialize a stored checkpoint into a fresh folder. Nothing is overwritten.</p>
        </div>
      </div>
      <div class="card-body">
        <form data-recovery-form>
          <div class="form-grid">
            <label class="field">
              <span class="field-label">Project</span>
              <select class="select-input" name="project" data-recovery-project>
                ${list
                  .map(
                    (project) =>
                      `<option value="${escapeHtml(project.project_id)}"${
                        recoveryProject?.project_id === project.project_id ? " selected" : ""
                      }>${escapeHtml(project.display_name)}</option>`
                  )
                  .join("")}
              </select>
            </label>
            <label class="field">
              <span class="field-label">Checkpoint</span>
              <select class="select-input" name="snapshot" data-recovery-snapshot>
                ${recoverySnapshots
                  .map(
                    (snapshot) =>
                      `<option value="${escapeHtml(snapshot.snapshot_id)}"${
                        chosenSnapshot?.snapshot_id === snapshot.snapshot_id ? " selected" : ""
                      }>#${snapshot.sequence_number} · ${escapeHtml(formatAge(snapshot.created_at_unix_seconds))}${
                        snapshot.label ? ` · ${escapeHtml(snapshot.label)}` : ""
                      }</option>`
                  )
                  .join("")}
              </select>
            </label>
            <label class="field">
              <span class="field-label">Recovery path</span>
              <input class="text-input" name="path" placeholder="/path/for/recovered-work" required />
            </label>
            <label class="field">
              <span class="field-label">Name (optional)</span>
              <input class="text-input" name="name" placeholder="recovered-attempt" />
            </label>
          </div>
          <div class="form-actions" style="margin-top: 10px;">
            <label class="check-row"><input type="checkbox" name="register" /> Register recovered folder as a workspace</label>
            <button class="button" type="submit">Open recovery</button>
          </div>
        </form>
      </div>
    </section>`;
}

function renderProjects() {
  const visible = filteredProjects();
  const attention = visible.filter((project) => projectNeedsAttention(project));
  const ready = visible.filter((project) => !projectNeedsAttention(project));
  return `
    <div class="page-head">
      <div>
        <div class="eyebrow"><span class="eyebrow-dot"></span>${plural(projects().length, "registered project")}</div>
        <h1 class="page-title">Projects</h1>
      </div>
      <div class="page-actions">
        <label class="filter-input">
          ${icon("search")}
          <input type="text" data-project-filter value="${escapeHtml(state.projectFilter)}" placeholder="Filter projects" aria-label="Filter projects" />
        </label>
      </div>
    </div>
    ${
      visible.length === 0
        ? `<div class="card"><div class="card-body"><div class="empty">${icon("folder")}<strong>No matching projects</strong><span>Adjust the filter or add a project below.</span></div></div></div>`
        : `<section class="card"><div class="card-body">
            ${renderProjectGroup("Needs attention", attention)}
            ${renderProjectGroup("Ready", ready)}
          </div></section>`
    }
    <div class="bento">
      <div class="bento-col">
        <section class="card">
          <div class="card-head">
            <div>
              <h3 class="card-title">${icon("folder")}Add project</h3>
              <p class="card-sub">Register an existing Git project with the local agent.</p>
            </div>
          </div>
          <div class="card-body">
            <form data-add-project-form>
              <div class="form-grid">
                <label class="field">
                  <span class="field-label">Project path</span>
                  <input class="text-input" name="path" placeholder="/path/to/project" required />
                </label>
                <label class="field">
                  <span class="field-label">Manifest path (optional)</span>
                  <input class="text-input" name="manifest" placeholder="devrelay.toml" />
                </label>
              </div>
              <div class="form-actions" style="margin-top: 10px;">
                <button class="button primary" type="submit">Add project</button>
                ${pendingAction("projects.pick-folder", "Browse folders", "small ghost")}
              </div>
            </form>
          </div>
        </section>
      </div>
      <div class="bento-col">
        ${renderRecoveryForm()}
      </div>
    </div>`;
}

/* -------------------------------------------------------- devices view */

function renderResourceTiles(device) {
  const summary = device.resource_summary;
  if (!summary) return "";
  const tiles = [
    ["cpu", "CPU", summary.cpu],
    ["box", "Memory", summary.memory],
    ["download", "Disk", summary.disk],
    ["zap", "Power", summary.power],
    ["history", "Cache", summary.cache_warmth],
  ].filter(([, , value]) => Boolean(value));
  if (tiles.length === 0) return "";
  return `<div class="resource-grid">${tiles
    .map(
      ([iconName, label, value]) => `
      <div class="resource-tile">
        <span class="resource-kind">${icon(iconName)}${label}</span>
        <div class="resource-value">${escapeHtml(value)}</div>
      </div>`
    )
    .join("")}</div>`;
}

function renderDeviceCard(device) {
  const online = deviceOnline(device);
  const isLocal = device.device_id === localDeviceId();
  const name = escapeHtml(device.display_name);
  const capabilities = deviceCapabilities(device);
  return `
    <article class="device-card">
      <div class="device-card-head">
        <div class="device-glyph ${deviceGlyphTone(device)}">${icon(deviceGlyphIcon(device))}</div>
        <div class="device-title">
          <strong>${name}</strong>
          <span>${escapeHtml(deviceOsFamily(device))} · ${escapeHtml(device.architecture ?? "unknown")} · seen ${escapeHtml(
            formatAge(device.last_seen_unix_seconds)
          )}</span>
        </div>
        ${
          isLocal
            ? '<span class="chip violet"><span class="chip-dot"></span>This device</span>'
            : online
              ? '<span class="chip good"><span class="chip-dot"></span>Online</span>'
              : '<span class="chip"><span class="chip-dot"></span>Offline</span>'
        }
      </div>
      ${renderResourceTiles(device)}
      ${
        capabilities.length > 0
          ? `<div class="capability-row">${capabilities.map((value) => `<span class="capability">${escapeHtml(value)}</span>`).join("")}</div>`
          : ""
      }
      <div class="device-card-actions">
        ${
          isLocal
            ? ""
            : `<button class="button small" data-view="continue" aria-label="Continue on ${name}">Continue on</button>`
        }
        ${pendingAction("devices.revoke", "Revoke", "small ghost", `Revoke ${device.display_name}`)}
      </div>
    </article>`;
}

function renderDevices() {
  const list = devices();
  const online = list.filter((device) => deviceOnline(device)).length;
  return `
    <div class="page-head">
      <div>
        <div class="eyebrow"><span class="eyebrow-dot"></span>Trusted fabric devices</div>
        <h1 class="page-title">Devices</h1>
        <p class="page-sub">${list.length} known identities - ${online} online. Identity, capability, and freshness come from the agent registry.</p>
      </div>
      <div class="page-actions">
        ${pendingAction("devices.policy", "Resource policy", "ghost")}
        ${pendingAction("devices.pair", "Pair device", "primary")}
      </div>
    </div>
    ${
      list.length === 0
        ? `<div class="card"><div class="card-body"><div class="empty">${icon("monitor")}<strong>No devices known</strong><span>Pair devices with the CLI until desktop pairing lands.</span></div></div></div>`
        : `<div class="device-grid">${list.map((device) => renderDeviceCard(device)).join("")}</div>`
    }`;
}

/* ----------------------------------------------------------- runs view */

function runTone(runState) {
  if (runState === "succeeded") return "good";
  if (runState === "running") return "violet";
  if (runState === "queued") return "info";
  if (runState === "failed") return "bad";
  return "";
}

function renderRunRow(run) {
  const note = schedulerNote(run);
  const targetId = runTargetDeviceId(run);
  const artifactCount = runArtifactCount(run);
  const active = run.state === "queued" || run.state === "running";
  return `
    <div class="device-row">
      <div class="feed-icon ${runTone(run.state)}">${icon(run.state === "failed" ? "x" : run.state === "succeeded" ? "check" : "play")}</div>
      <div class="device-row-main">
        <strong class="mono">${escapeHtml(run.command ?? run.task_run_id)}</strong>
        <span>${escapeHtml(projectName(run.project_id))}${targetId ? ` · on ${escapeHtml(deviceName(targetId))}` : ""} · ${escapeHtml(
          formatAge(run.updated_at_unix_seconds)
        )}${note ? `<br />Scheduler explanation: ${escapeHtml(note)}` : ""}</span>
      </div>
      <div class="row-actions">
        ${
          active
            ? pendingAction("runs.cancel", "Cancel", "small ghost", `Cancel run ${run.task_run_id}`)
            : pendingAction("runs.artifacts", `Artifacts (${artifactCount})`, "small ghost", `Artifacts for run ${run.task_run_id}`)
        }
      </div>
    </div>`;
}

function renderRunGroup(label, list) {
  return `
    <p class="group-label">${escapeHtml(label)} (${list.length})</p>
    ${
      list.length > 0
        ? list.map((run) => renderRunRow(run)).join("")
        : '<p class="quiet">None right now.</p>'
    }`;
}

function renderRuns() {
  const queued = runsByState("queued");
  const running = runsByState("running");
  const failed = runsByState("failed");
  const finished = runsByState("succeeded");
  return `
    <div class="page-head">
      <div>
        <div class="eyebrow"><span class="eyebrow-dot"></span>Distributed task runner</div>
        <h1 class="page-title">Runs</h1>
        <p class="page-sub">Recent runs recorded by the agent, with the reason each target was chosen.</p>
      </div>
      <div class="page-actions">
        ${pendingAction("runs.cache-history", "Cache history", "ghost")}
        ${pendingAction("runs.start", "Run task", "primary")}
      </div>
    </div>
    <div class="bento">
      <div class="bento-col">
        <section class="card">
          <div class="card-head">
            <div>
              <h3 class="card-title">${icon("terminal")}Recent runs</h3>
              <p class="card-sub">${plural(runs().length, "recorded run")}</p>
            </div>
          </div>
          <div class="card-body">
            ${
              runs().length === 0
                ? `<div class="empty">${icon("terminal")}<strong>No runs yet</strong><span>Task history appears once the fabric executes work.</span></div>`
                : `
                  ${renderRunGroup("Running runs", running)}
                  ${renderRunGroup("Queued runs", queued)}
                  ${renderRunGroup("Failed runs", failed)}
                  ${renderRunGroup("Completed runs", finished)}`
            }
          </div>
        </section>
      </div>
      <div class="bento-col">
        <section class="card">
          <div class="card-head">
            <div>
              <h3 class="card-title">${icon("sliders")}Scheduler</h3>
              <p class="card-sub">Explainable target selection.</p>
            </div>
            ${pendingChip("scheduler.controls")}
          </div>
          <div class="card-body">
            ${
              runs().filter((run) => schedulerNote(run)).length > 0
                ? `<ul class="check-list">${runs()
                    .filter((run) => schedulerNote(run))
                    .slice(0, 5)
                    .map(
                      (run) =>
                        `<li class="check-item">${icon("check")}<span>${escapeHtml(schedulerNote(run))}<span class="check-note">${escapeHtml(
                          run.command ?? run.task_run_id
                        )}</span></span></li>`
                    )
                    .join("")}</ul>`
                : '<p class="muted">No scheduler decisions recorded yet.</p>'
            }
            ${pendingPanel("runs.cache-history", "Result-cache hit rates and history charts arrive with the compute fabric UI.")}
          </div>
        </section>
      </div>
    </div>`;
}

/* ------------------------------------------------------- activity view */

const activityFilters = [
  ["all", "All"],
  ["audit", "Audit"],
  ["checkpoint", "Checkpoints"],
  ["handoff", "Handoffs"],
  ["security", "Security"],
  ["quota", "Quota"],
];

function activitySectionVisible(key) {
  return state.activityFilter === "all" || state.activityFilter === key;
}

function feedItem(tone, iconName, title, sub, timeSeconds) {
  return `
    <div class="feed-item">
      <div class="feed-icon ${tone}">${icon(iconName)}</div>
      <div class="feed-main">
        <div class="feed-title">${title}</div>
        ${sub ? `<div class="feed-sub">${sub}</div>` : ""}
      </div>
      <div class="feed-time">${escapeHtml(timeSeconds ? formatAge(timeSeconds) : "")}</div>
    </div>`;
}

function renderActivitySection(key, title, iconName, itemsHtml, emptyCopy) {
  if (!activitySectionVisible(key)) return "";
  return `
    <section class="card">
      <div class="card-head">
        <div>
          <h3 class="card-title">${icon(iconName)}${title}</h3>
        </div>
      </div>
      <div class="card-body">
        <div class="feed">${itemsHtml || `<div class="empty">${icon(iconName)}<strong>${emptyCopy}</strong></div>`}</div>
      </div>
    </section>`;
}

function renderActivity() {
  const auditItems = activity()
    .slice(0, 30)
    .map((entry) =>
      feedItem(
        entry.outcome === "succeeded" ? "good" : entry.outcome === "failed" ? "bad" : "info",
        "pulse",
        `${escapeHtml(titleize(String(entry.type ?? "").replaceAll(".", " ")))} · ${escapeHtml(titleize(entry.outcome ?? ""))}`,
        `${escapeHtml(entry.summary ?? "")}${entry.project_id ? ` · ${escapeHtml(projectName(entry.project_id))}` : ""}`,
        entry.created_at_unix_seconds
      )
    )
    .join("");
  const checkpointItems = checkpointEvents()
    .map((event) =>
      feedItem(
        "good",
        "shield",
        escapeHtml(snapshotEventSummary(event)),
        `${escapeHtml(projectName(event.payload?.project_id))}${
          event.payload?.snapshot_sequence_number ? ` · #${event.payload.snapshot_sequence_number}` : ""
        }${event.payload?.label ? ` · ${escapeHtml(event.payload.label)}` : ""}`,
        event.occurredAt ? Math.floor(event.occurredAt / 1000) : null
      )
    )
    .join("");
  const handoffItems = handoffEvents()
    .map((event) =>
      feedItem(
        "info",
        "relay",
        `Handoff ${escapeHtml(titleize(event.payload?.state ?? ""))}`,
        `${escapeHtml(projectName(event.payload?.project_id))} · ${escapeHtml(deviceName(event.payload?.source_device_id))} &rarr; ${escapeHtml(
          deviceName(event.payload?.target_device_id)
        )}`,
        event.occurredAt ? Math.floor(event.occurredAt / 1000) : null
      )
    )
    .join("");
  const securityItems = securityEvents()
    .map((event) =>
      feedItem(
        "bad",
        "lock",
        escapeHtml(event.payload?.title ?? "Security block"),
        `${escapeHtml(event.payload?.detail ?? "")}${
          Array.isArray(event.payload?.safe_actions) && event.payload.safe_actions.length > 0
            ? ` · Safe next step: ${escapeHtml(event.payload.safe_actions[0])}`
            : ""
        }`,
        event.occurredAt ? Math.floor(event.occurredAt / 1000) : null
      )
    )
    .join("");
  const quotaItems = quotaEvents()
    .map((event) =>
      feedItem(
        "warn",
        "zap",
        `Quota warning · ${escapeHtml(event.payload?.quota ?? "")}`,
        `${escapeHtml(String(event.payload?.used ?? "?"))}/${escapeHtml(String(event.payload?.limit ?? "?"))} ${escapeHtml(
          event.payload?.unit ?? ""
        )} · ${escapeHtml(event.payload?.detail ?? "")}`,
        event.occurredAt ? Math.floor(event.occurredAt / 1000) : null
      )
    )
    .join("");
  return `
    <div class="page-head">
      <div>
        <div class="eyebrow"><span class="eyebrow-dot"></span>Local audit trail</div>
        <h1 class="page-title">Activity</h1>
        <p class="page-sub">Evidence for checkpoints, handoffs, security blocks, and quota pressure. Internal identifiers stay in exported diagnostics only.</p>
      </div>
      <div class="page-actions">
        <button class="button" data-action="diagnostics" aria-label="Export diagnostics bundle">${icon("download")}Diagnostics</button>
      </div>
    </div>
    <div class="card">
      <div class="card-body">
        <p class="group-label">Activity filters</p>
        <div class="segmented" role="group" aria-label="Activity filters">
          ${activityFilters
            .map(
              ([key, label]) =>
                `<button class="segment${state.activityFilter === key ? " active" : ""}" data-activity-filter="${key}">${label}</button>`
            )
            .join("")}
        </div>
      </div>
    </div>
    ${renderActivitySection("audit", "Audit events", "pulse", auditItems, "No audit events yet")}
    ${renderActivitySection("checkpoint", "Checkpoint events", "shield", checkpointItems, "No live checkpoint events yet")}
    ${renderActivitySection("handoff", "Handoff events", "relay", handoffItems, "No live handoff events yet")}
    ${renderActivitySection("security", "Security blocks", "lock", securityItems, "No security blocks in this session")}
    ${renderActivitySection("quota", "Quota warnings", "zap", quotaItems, "No quota warnings in this session")}`;
}

/* ------------------------------------------------------- settings view */

const profileNotes = {
  adaptive: "Balance protection and resource use automatically.",
  instant: "Checkpoint aggressively for the fastest continue.",
  eco: "Pause background work to save power.",
  custom: "Use manually tuned thresholds.",
  balanced: "Steady cadence between instant and eco.",
  performance: "Prioritize local machine performance.",
};

function validateSettingsInput(formData) {
  const profile = String(formData.get("resource_profile") ?? "").trim();
  if (!resourceProfiles.includes(profile)) {
    return "Resource profile must be one of the supported profiles";
  }
  const editorCommand = String(formData.get("editor_command") ?? "").trim();
  if (!editorCommand) {
    return "Editor command is required";
  }
  return null;
}

function renderSettings() {
  const config = settings();
  const runtime = state.bootstrap?.runtime ?? null;
  const bridge = state.eventBridge;
  const warmth = localCacheWarmth();
  return `
    <div class="page-head">
      <div>
        <div class="eyebrow"><span class="eyebrow-dot"></span>Fabric policy</div>
        <h1 class="page-title">Settings</h1>
        <p class="page-sub">Local device preferences. Everything here is stored by the agent, not the window.</p>
      </div>
    </div>
    <form data-settings-form>
      <div class="bento">
        <div class="bento-col">
          <section class="card">
            <div class="card-head"><div><h3 class="card-title">${icon("monitor")}Device identity</h3></div></div>
            <div class="card-body">
              <dl class="kv-list">
                <div class="kv"><dt>Fabric</dt><dd>${escapeHtml(config?.fabric_name ?? "unknown")}</dd></div>
                <div class="kv"><dt>Device name</dt><dd>${escapeHtml(config?.device_name ?? "unknown")}</dd></div>
                <div class="kv"><dt>Device ID</dt><dd class="mono">${escapeHtml(shortId(config?.device_id))}</dd></div>
                <div class="kv"><dt>Platform</dt><dd>${escapeHtml(config?.platform_key ?? runtime?.platform_key ?? "unknown")} · ${escapeHtml(
                  config?.architecture ?? runtime?.architecture ?? ""
                )}</dd></div>
                <div class="kv"><dt>Registered projects</dt><dd>${config?.project_count ?? projects().length}</dd></div>
              </dl>
            </div>
          </section>
          <section class="card">
            <div class="card-head">
              <div>
                <h3 class="card-title">${icon("zap")}Background behavior</h3>
                <p class="card-sub">How eagerly the agent protects work in the background.</p>
              </div>
              ${pendingChip("settings.standby-target")}
            </div>
            <div class="card-body">
              <label class="field">
                <span class="field-label">Resource profile</span>
                <select class="select-input" name="resource_profile">
                  ${resourceProfiles
                    .map(
                      (profile) =>
                        `<option value="${profile}"${config?.resource_profile === profile ? " selected" : ""}>${titleize(profile)} — ${
                          profileNotes[profile]
                        }</option>`
                    )
                    .join("")}
                </select>
              </label>
              ${pendingPanel("settings.standby-target", "Pinning a default continue target arrives with scheduler preferences.")}
            </div>
          </section>
          <section class="card">
            <div class="card-head">
              <div>
                <h3 class="card-title">${icon("box")}Storage and cache</h3>
                <p class="card-sub">Where checkpoints and metadata live on this device.</p>
              </div>
              ${pendingChip("settings.retention")}
            </div>
            <div class="card-body">
              <dl class="kv-list">
                <div class="kv"><dt>Data home</dt><dd class="mono">${escapeHtml(runtime?.devrelay_home ?? "unknown")}</dd></div>
                <div class="kv"><dt>Checkpoint cache</dt><dd>${escapeHtml(warmth ?? `${snapshots().length} checkpoints stored`)}</dd></div>
              </dl>
              ${pendingPanel("settings.retention", "Retention windows and per-project quota controls stay on agent defaults for now.")}
            </div>
          </section>
        </div>
        <div class="bento-col">
          <section class="card">
            <div class="card-head"><div><h3 class="card-title">${icon("wifi")}Network</h3><p class="card-sub">Discovery and transport between your devices.</p></div></div>
            <div class="card-body">
              <label class="check-row">
                <input type="checkbox" name="mdns_enabled" ${config?.mdns_enabled ? "checked" : ""} />
                Discover devices on the local network (mDNS)
              </label>
              <div class="kv"><dt class="muted">Agent endpoint</dt><dd class="mono">${escapeHtml(shortId(runtime?.agent_socket_path))}</dd></div>
            </div>
          </section>
          <section class="card">
            <div class="card-head"><div><h3 class="card-title">${icon("lock")}Security</h3><p class="card-sub">Trust decisions stay with the agent and the CLI.</p></div></div>
            <div class="card-body">
              <dl class="kv-list">
                <div class="kv"><dt>Anchor mode</dt><dd>${escapeHtml(titleize(config?.anchor_mode ?? "unknown"))}</dd></div>
                <div class="kv"><dt>Device trust</dt><dd>Managed via pairing and revocation</dd></div>
              </dl>
              <p class="quiet">Security blocks appear in Activity with safe next steps; secrets are excluded from checkpoints by policy.</p>
            </div>
          </section>
          <section class="card">
            <div class="card-head">
              <div>
                <h3 class="card-title">${icon("terminal")}Editor context</h3>
                <p class="card-sub">How projects open after a continue.</p>
              </div>
              ${pendingChip("editor.context-restore")}
            </div>
            <div class="card-body">
              <label class="field">
                <span class="field-label">Editor command</span>
                <input class="text-input" name="editor_command" value="${escapeHtml(config?.editor_command ?? "")}" placeholder="code" />
                <span class="field-hint">Used by Open in editor and after a verified continue.</span>
              </label>
              ${pendingPanel("editor.context-restore", "Tab, breakpoint, and terminal restore is driven from the VS Code extension today.")}
            </div>
          </section>
          <section class="card">
            <div class="card-head"><div><h3 class="card-title">${icon("sliders")}Appearance</h3></div></div>
            <div class="card-body">
              <div class="segmented" role="group" aria-label="Color theme">
                ${["auto", "light", "dark"]
                  .map(
                    (theme) =>
                      `<button type="button" class="segment${state.theme === theme ? " active" : ""}" data-action="set-theme" data-theme-value="${theme}">${titleize(
                        theme
                      )}</button>`
                  )
                  .join("")}
              </div>
              <p class="quiet">Theme follows the system in Auto and is not persisted yet.</p>
            </div>
          </section>
          <section class="card">
            <div class="card-head"><div><h3 class="card-title">${icon("pulse")}Advanced diagnostics</h3><p class="card-sub">Support evidence, without internal identifiers in the UI.</p></div></div>
            <div class="card-body">
              <dl class="kv-list">
                <div class="kv"><dt>Event stream</dt><dd>${escapeHtml(eventBridgeStatus().label)}</dd></div>
                <div class="kv"><dt>Last live event</dt><dd>${escapeHtml(bridge.lastEvent ? formatClock(bridge.lastEvent.receivedAt) : "none yet")}</dd></div>
              </dl>
              <div class="form-actions">
                <button class="button" type="button" data-action="diagnostics">${icon("download")}Export diagnostics</button>
              </div>
            </div>
          </section>
        </div>
      </div>
      <div class="form-actions" style="margin-top: 14px;">
        <button class="button primary" type="submit">Save settings</button>
        <span class="quiet">Applies through the agent settings RPC.</span>
      </div>
    </form>`;
}

/* ------------------------------------------------------ command palette */

function paletteCommands() {
  const commands = [];
  for (const [id, label, iconName] of views) {
    commands.push({ group: "Go to", label: `Go to ${label}`, iconName, kind: "view", view: id });
  }
  const project = selectedProject();
  if (project) {
    commands.push({
      group: "Actions",
      label: `Checkpoint ${project.display_name} now`,
      iconName: "shield",
      kind: "action",
      action: "checkpoint",
      projectId: project.project_id,
    });
    const target = bestTarget();
    if (target) {
      commands.push({
        group: "Actions",
        label: `Review handoff to ${target.display_name}`,
        iconName: "arrow-right",
        kind: "action",
        action: "handoff-dialog",
        projectId: project.project_id,
        targetDeviceId: target.device_id,
      });
    }
    commands.push({
      group: "Actions",
      label: `Open ${project.display_name} in editor`,
      iconName: "terminal",
      kind: "action",
      action: "open-project",
      projectId: project.project_id,
    });
  }
  commands.push({ group: "Actions", label: "Export diagnostics", iconName: "download", kind: "action", action: "diagnostics" });
  commands.push({ group: "Actions", label: "Refresh state", iconName: "history", kind: "action", action: "refresh" });
  commands.push({ group: "Actions", label: "Switch color theme", iconName: "sun", kind: "action", action: "toggle-theme" });
  for (const [featureId, entry] of Object.entries(FEATURES)) {
    commands.push({
      group: "Not built yet",
      label: entry.title,
      iconName: "cone",
      kind: "pending",
      featureId,
    });
  }
  return commands;
}

function filteredPaletteCommands() {
  const query = (state.palette?.query ?? "").trim().toLowerCase();
  const commands = paletteCommands();
  if (!query) return commands;
  return commands.filter((command) => command.label.toLowerCase().includes(query));
}

function renderPalette() {
  if (!state.palette) return "";
  const commands = filteredPaletteCommands();
  const selected = Math.min(state.palette.index ?? 0, Math.max(0, commands.length - 1));
  let lastGroup = null;
  const items = commands
    .map((command, index) => {
      const groupHtml = command.group !== lastGroup ? `<div class="palette-group">${escapeHtml(command.group)}</div>` : "";
      lastGroup = command.group;
      return `${groupHtml}
        <button class="palette-item${index === selected ? " selected" : ""}" data-action="palette-run" data-palette-index="${index}">
          ${icon(command.iconName)}
          <span class="palette-item-label">${escapeHtml(command.label)}</span>
          ${command.kind === "pending" ? pendingChip(command.featureId) : ""}
        </button>`;
    })
    .join("");
  return `
    <div class="overlay palette-overlay" data-overlay="palette">
      <div class="palette" role="dialog" aria-modal="true" aria-label="Command menu">
        <div class="palette-input-row">
          ${icon("search")}
          <input class="palette-input" data-palette-input type="text" value="${escapeHtml(state.palette.query ?? "")}"
            placeholder="Search views, actions, projects" aria-label="Search commands" />
          <kbd>esc</kbd>
        </div>
        <div class="palette-list">
          ${commands.length > 0 ? items : '<div class="palette-empty">No matching commands.</div>'}
        </div>
      </div>
    </div>`;
}

/* ------------------------------------------------------- shell + render */

function renderToasts() {
  if (state.toasts.length === 0) return "";
  return `
    <div class="toast-region" role="status" aria-live="polite">
      ${state.toasts
        .map(
          (entry) => `
        <div class="toast ${entry.kind}">
          ${icon(entry.kind === "bad" ? "x" : entry.kind === "warn" ? "cone" : "check")}
          <div>${escapeHtml(entry.message)}</div>
        </div>`
        )
        .join("")}
    </div>`;
}

function agentNotices() {
  const notices = [];
  const bridge = state.eventBridge;
  const agent = state.bootstrap?.agent;
  if (state.bootstrap && agent && !agent.connected) {
    notices.push(
      `<div class="banner bad">${icon("x")}<div class="banner-body"><strong>Local agent is not reachable</strong><span>Start the DevRelay agent, then refresh. State shown below may be stale.</span></div></div>`
    );
  }
  if (bridge.stale) {
    notices.push(
      `<div class="banner warn">${icon("cone")}<div class="banner-body"><strong>Event stream gap</strong><span>Some live events were missed; resyncing full state from the agent.</span></div></div>`
    );
  }
  if (!bridge.connected && bridge.everConnected) {
    notices.push(
      `<div class="banner warn">${icon("cone")}<div class="banner-body"><strong>Events reconnecting</strong><span>The live event stream dropped; reconnecting in the background.</span></div></div>`
    );
  }
  for (const error of (agent?.errors ?? []).slice(0, 3)) {
    notices.push(
      `<div class="banner warn">${icon("cone")}<div class="banner-body"><strong>Agent notice</strong><span>${escapeHtml(error)}</span></div></div>`
    );
  }
  return notices.join("");
}

function loadingScreen() {
  return `<div class="card"><div class="card-body"><div class="empty">${icon("relay")}<strong>Connecting to the local runtime</strong><span>Loading agent state.</span></div></div></div>`;
}

function runtimeErrorScreen() {
  return `
    <div class="banner bad">${icon("x")}<div class="banner-body"><strong>Desktop runtime problem</strong><span>${escapeHtml(
      state.runtimeError ?? "Unknown problem"
    )}</span></div></div>
    <div class="form-actions"><button class="button" data-action="refresh">Try again</button></div>`;
}

function renderView() {
  switch (state.view) {
    case "projects":
      return renderProjects();
    case "devices":
      return renderDevices();
    case "runs":
      return renderRuns();
    case "activity":
      return renderActivity();
    case "settings":
      return renderSettings();
    default:
      return renderContinue();
  }
}

function shell() {
  return `
    <div class="app-root" data-theme="${resolvedTheme()}">
      <div class="ambient" aria-hidden="true"></div>
      <div class="shell">
        ${renderSidebar()}
        <div class="workspace">
          ${renderTopbar()}
          <main class="main">
            <div class="view-frame">
              ${agentNotices()}
              ${state.loading ? loadingScreen() : state.runtimeError ? runtimeErrorScreen() : renderView()}
            </div>
          </main>
        </div>
      </div>
      ${renderHandoffDialog()}
      ${renderPalette()}
      ${renderToasts()}
    </div>`;
}

function render() {
  app.innerHTML = shell();
  attachHandlers();
  applyPendingFocus();
}

/* -------------------------------------------------- keyboard and focus */

const dialogFocusableSelector =
  'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

function closeHandoffDialog() {
  const returnFocusSelector = state.handoffDialog?.returnFocusSelector ?? null;
  state.handoffDialog = null;
  queueFocus(returnFocusSelector);
  render();
}

function closePalette() {
  state.palette = null;
  queueFocus('[data-action="palette-open"]');
  render();
}

function trapHandoffDialogFocus(event) {
  const dialog = app.querySelector?.("[data-handoff-dialog]");
  if (!dialog) return;
  const focusable = [...dialog.querySelectorAll(dialogFocusableSelector)].filter(
    (element) => !element.disabled
  );
  if (focusable.length === 0) return;
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  const active = document.activeElement;
  if (event.shiftKey && active === first) {
    event.preventDefault?.();
    last.focus?.();
  } else if (!event.shiftKey && active === last) {
    event.preventDefault?.();
    first.focus?.();
  }
}

function handleGlobalKeydown(event) {
  if ((event.key === "k" || event.key === "K") && (event.metaKey || event.ctrlKey)) {
    event.preventDefault?.();
    if (state.palette) {
      closePalette();
    } else {
      state.palette = { query: "", index: 0 };
      queueFocus("[data-palette-input]");
      render();
    }
    return;
  }
  if (event.key === "Escape") {
    if (state.palette) {
      event.preventDefault?.();
      closePalette();
      return;
    }
    if (state.handoffDialog) {
      event.preventDefault?.();
      closeHandoffDialog();
    }
    return;
  }
  if (event.key === "Tab" && state.handoffDialog) {
    trapHandoffDialogFocus(event);
  }
}

/* ----------------------------------------------------- agent data flow */

function expectOk(result) {
  if (!result || result.ok !== true) {
    throw new Error(result?.message || "The agent turned down the request");
  }
  return result;
}

async function refresh() {
  if (!state.bootstrap) {
    state.loading = true;
    render();
  } else {
    state.eventBridge.refreshing = true;
    render();
  }
  try {
    const bootstrap = await invoke("ui_bootstrap");
    state.bootstrap = bootstrap;
    state.loading = false;
    state.runtimeError = null;
    state.eventBridge.refreshing = false;
    state.eventBridge.stale = false;
    if (!state.selectedProjectId && projects().length > 0) {
      state.selectedProjectId = projects()[0].project_id;
    }
    render();
    const project = selectedProject();
    if (project && !statusFor(project.project_id)) {
      await refreshProjectStatus(project.project_id, false);
    }
  } catch (error) {
    state.loading = false;
    state.eventBridge.refreshing = false;
    state.runtimeError = error?.message ?? String(error);
    render();
  }
}

async function refreshProjectStatus(projectId, announce = true) {
  if (!projectId) return;
  state.projectStatus.set(projectId, {
    loading: true,
    error: null,
    data: statusFor(projectId)?.data ?? null,
  });
  render();
  try {
    const result = await invoke("project_status", { projectId });
    const data = result?.ok === true ? result.data : null;
    if (!data) {
      throw new Error(result?.message || "Status request failed");
    }
    state.projectStatus.set(projectId, { loading: false, error: null, data });
    if (announce) toast("Status refreshed from the agent");
    render();
  } catch (error) {
    state.projectStatus.set(projectId, {
      loading: false,
      error: error?.message ?? String(error),
      data: null,
    });
    if (announce) toast(error?.message ?? "Status request failed", "bad");
    render();
  }
}

/* --------------------------------------------------------- event bridge */

let pendingEventRefresh = null;

function queueEventRefresh(delay) {
  if (pendingEventRefresh !== null) {
    window.clearTimeout(pendingEventRefresh);
  }
  state.eventBridge.refreshing = true;
  render();
  pendingEventRefresh = window.setTimeout(() => {
    pendingEventRefresh = null;
    refresh();
  }, delay);
}

function markEventBridgeConnected(payload) {
  const bridge = state.eventBridge;
  bridge.connected = true;
  bridge.everConnected = true;
  bridge.lastConnectedAt = Date.now();
  bridge.lastError = null;
  bridge.subscription = {
    replayed: payload?.replayed ?? 0,
    currentSequence: payload?.current_sequence ?? null,
    cursorSequence: payload?.cursor?.after_sequence ?? null,
  };
  render();
}

function markEventBridgeEvent(payload) {
  const bridge = state.eventBridge;
  const entry = {
    sequence: payload?.sequence ?? null,
    type: payload?.type ?? "unknown",
    payload: payload?.payload ?? {},
    occurredAt: payload?.occurred_at_unix_millis ?? null,
    receivedAt: Date.now(),
  };
  bridge.lastEvent = entry;
  bridge.events = [entry, ...bridge.events.filter((event) => event.sequence !== entry.sequence)].slice(0, 100);
  render();
}

function markEventBridgeGap(payload) {
  const bridge = state.eventBridge;
  bridge.connected = true;
  bridge.everConnected = true;
  bridge.stale = true;
  bridge.lastGap = {
    expectedAfter: payload?.expected_after ?? null,
    actualNext: payload?.actual_next ?? null,
  };
  render();
}

function markEventBridgeDisconnected(payload) {
  const bridge = state.eventBridge;
  bridge.connected = false;
  bridge.refreshing = false;
  bridge.lastDisconnectedAt = Date.now();
  bridge.lastError = typeof payload === "string" ? payload : payload?.message ?? "Event stream closed";
  render();
}

/* -------------------------------------------------------------- actions */

async function handleAction(button) {
  const action = button?.dataset?.action;
  if (!action) return;
  const projectId = button.dataset.projectId ?? null;
  const targetDeviceId = button.dataset.targetDeviceId ?? null;
  const handoffId = button.dataset.handoffId ?? null;
  if (action === "handoff-dialog-close") {
    closeHandoffDialog();
    return;
  }
  if (action === "handoff-dialog") {
    state.handoffDialog = {
      projectId,
      targetDeviceId,
      phase: "review",
      message: null,
      returnFocusSelector: actionFocusSelector(button),
    };
    queueFocus('[data-action="handoff-dialog-close"]');
    render();
    if (projectId && !statusFor(projectId)) {
      await refreshProjectStatus(projectId, false);
    }
    return;
  }
  if (action === "feature-pending") {
    const entry = featureEntry(button.dataset.feature);
    toast(entry?.toast ?? "This part of the product is not built yet", "warn");
    return;
  }
  if (action === "refresh") {
    await refresh();
    return;
  }
  if (action === "open-activity") {
    state.view = "activity";
    render();
    return;
  }
  if (action === "toggle-theme") {
    state.theme = state.theme === "auto" ? "light" : state.theme === "light" ? "dark" : "auto";
    render();
    return;
  }
  if (action === "set-theme") {
    state.theme = button.dataset.themeValue ?? "auto";
    render();
    return;
  }
  if (action === "palette-open") {
    state.palette = { query: "", index: 0 };
    queueFocus("[data-palette-input]");
    render();
    return;
  }
  if (action === "palette-run") {
    await runPaletteCommand(Number(button.dataset.paletteIndex ?? 0));
    return;
  }
  if (action === "project-status") {
    await refreshProjectStatus(projectId);
    return;
  }
  if (action === "project-recovery") {
    state.recoveryProjectId = projectId;
    state.recoverySnapshotId = latestSnapshot(projectId)?.snapshot_id ?? null;
    state.view = "projects";
    queueFocus("[data-recovery-snapshot]");
    render();
    return;
  }
  if (action === "checkpoint") {
    try {
      const result = expectOk(await invoke("checkpoint_create", { projectId }));
      toast(result.message || "Checkpoint created");
      await refresh();
    } catch (error) {
      toast(error?.message ?? "Checkpoint failed", "bad");
    }
    return;
  }
  if (action === "handoff-confirm") {
    if (!state.handoffDialog) return;
    state.handoffDialog.phase = "running";
    render();
    try {
      const result = expectOk(await invoke("handoff_prepare", { projectId, targetDeviceId }));
      if (state.handoffDialog) {
        state.handoffDialog.phase = "success";
        state.handoffDialog.message =
          result.message ||
          "Target preparation has started. Continue from the target device once it reports ready.";
      }
      toast("Handoff preparation started");
      await refresh();
    } catch (error) {
      if (state.handoffDialog) {
        state.handoffDialog.phase = "failure";
        state.handoffDialog.message = error?.message ?? "Handoff could not start";
      }
      render();
    }
    return;
  }
  if (action === "handoff-abort") {
    try {
      const result = expectOk(await invoke("handoff_abort", { projectId, handoffId }));
      if (state.handoffDialog) {
        state.handoffDialog.phase = "success";
        state.handoffDialog.message = "Handoff aborted safely; nothing moved.";
      }
      toast(result.message || "Handoff aborted");
      await refresh();
    } catch (error) {
      toast(error?.message ?? "Handoff abort failed", "bad");
    }
    return;
  }
  if (action === "handoff-continue-here") {
    try {
      const result = expectOk(await invoke("handoff_continue_here", { projectId, handoffId }));
      toast(result.message || "Continuation verified");
      await refresh();
    } catch (error) {
      toast(error?.message ?? "Continue here failed", "bad");
    }
    return;
  }
  if (action === "open-project") {
    try {
      const result = expectOk(await invoke("open_project", { projectId }));
      toast(`Opened ${result.data ?? "project"}`);
    } catch (error) {
      toast(error?.message ?? "Open failed", "bad");
    }
    return;
  }
  if (action === "diagnostics") {
    try {
      const result = expectOk(await invoke("diagnostics_export"));
      toast(`Diagnostics exported to ${result.data?.path ?? "bundle"}`);
    } catch (error) {
      toast(error?.message ?? "Diagnostics export failed", "bad");
    }
  }
}

async function runPaletteCommand(index) {
  const commands = filteredPaletteCommands();
  const command = commands[index];
  state.palette = null;
  if (!command) {
    render();
    return;
  }
  if (command.kind === "view") {
    state.view = command.view;
    render();
    return;
  }
  if (command.kind === "pending") {
    const entry = featureEntry(command.featureId);
    toast(entry?.toast ?? "This part of the product is not built yet", "warn");
    return;
  }
  render();
  await handleAction({
    dataset: {
      action: command.action,
      projectId: command.projectId,
      targetDeviceId: command.targetDeviceId,
    },
  });
}

/* ------------------------------------------------------- DOM listeners */

function attachHandlers() {
  for (const button of app.querySelectorAll("[data-action]")) {
    button.addEventListener("click", () => {
      handleAction(button);
    });
  }
  for (const button of app.querySelectorAll("[data-view]")) {
    button.addEventListener("click", () => {
      state.view = button.dataset.view;
      render();
    });
  }
  for (const button of app.querySelectorAll("[data-project]")) {
    button.addEventListener("click", () => {
      state.selectedProjectId = button.dataset.project;
      state.view = "continue";
      render();
      refreshProjectStatus(button.dataset.project, false);
    });
  }
  for (const button of app.querySelectorAll("[data-activity-filter]")) {
    button.addEventListener("click", () => {
      state.activityFilter = button.dataset.activityFilter;
      render();
    });
  }
  const projectFilter = app.querySelector("[data-project-filter]");
  if (projectFilter) {
    projectFilter.addEventListener("input", () => {
      state.projectFilter = projectFilter.value;
      const caret = projectFilter.selectionStart ?? projectFilter.value.length;
      render();
      const next = app.querySelector("[data-project-filter]");
      next?.focus?.();
      next?.setSelectionRange?.(caret, caret);
    });
  }
  const paletteInput = app.querySelector("[data-palette-input]");
  if (paletteInput) {
    paletteInput.addEventListener("input", () => {
      if (!state.palette) return;
      state.palette.query = paletteInput.value;
      state.palette.index = 0;
      const caret = paletteInput.selectionStart ?? paletteInput.value.length;
      render();
      const next = app.querySelector("[data-palette-input]");
      next?.focus?.();
      next?.setSelectionRange?.(caret, caret);
    });
    paletteInput.addEventListener("keydown", (event) => {
      if (!state.palette) return;
      const total = filteredPaletteCommands().length;
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault?.();
        const delta = event.key === "ArrowDown" ? 1 : -1;
        state.palette.index = total === 0 ? 0 : (state.palette.index + delta + total) % total;
        render();
        const next = app.querySelector("[data-palette-input]");
        next?.focus?.();
      } else if (event.key === "Enter") {
        event.preventDefault?.();
        runPaletteCommand(state.palette.index ?? 0);
      }
    });
  }
  const recoveryProject = app.querySelector("[data-recovery-project]");
  if (recoveryProject) {
    recoveryProject.addEventListener("change", () => {
      state.recoveryProjectId = recoveryProject.value;
      state.recoverySnapshotId = latestSnapshot(recoveryProject.value)?.snapshot_id ?? null;
      render();
    });
  }
  const recoverySnapshot = app.querySelector("[data-recovery-snapshot]");
  if (recoverySnapshot) {
    recoverySnapshot.addEventListener("change", () => {
      state.recoverySnapshotId = recoverySnapshot.value;
    });
  }
  const settingsForm = app.querySelector("[data-settings-form]");
  if (settingsForm) {
    settingsForm.addEventListener("submit", async (event) => {
      event.preventDefault?.();
      const formData = new FormData(settingsForm);
      const problem = validateSettingsInput(formData);
      if (problem) {
        toast(problem, "bad");
        return;
      }
      try {
        expectOk(
          await invoke("settings_update", {
            params: {
              resource_profile: String(formData.get("resource_profile")),
              mdns_enabled: formData.get("mdns_enabled") !== null,
              editor_command: String(formData.get("editor_command") ?? "").trim(),
            },
          })
        );
        toast("Settings saved");
        await refresh();
      } catch (error) {
        toast(error?.message ?? "Settings update failed", "bad");
      }
    });
  }
  const addProjectForm = app.querySelector("[data-add-project-form]");
  if (addProjectForm) {
    addProjectForm.addEventListener("submit", async (event) => {
      event.preventDefault?.();
      const formData = new FormData(addProjectForm);
      const path = String(formData.get("path") ?? "").trim();
      const manifest = String(formData.get("manifest") ?? "").trim();
      if (!path) {
        toast("Project path is required", "bad");
        return;
      }
      try {
        const result = expectOk(await invoke("project_add", { path, manifest: manifest || null }));
        toast(result.message || "Project added");
        await refresh();
      } catch (error) {
        toast(error?.message ?? "Add project failed", "bad");
      }
    });
  }
  const recoveryForm = app.querySelector("[data-recovery-form]");
  if (recoveryForm) {
    recoveryForm.addEventListener("submit", async (event) => {
      event.preventDefault?.();
      const formData = new FormData(recoveryForm);
      const projectId = String(formData.get("project") ?? state.recoveryProjectId ?? "");
      const snapshotId = String(formData.get("snapshot") ?? state.recoverySnapshotId ?? "");
      const path = String(formData.get("path") ?? "").trim();
      const name = String(formData.get("name") ?? "").trim();
      const register = formData.get("register") !== null;
      if (!projectId || !snapshotId || !path) {
        toast("Recovery needs a project, a checkpoint, and a fresh path", "bad");
        return;
      }
      try {
        const result = expectOk(
          await invoke("recover_open", { projectId, snapshotId, path, name: name || null, register })
        );
        toast(result.message || `Recovered to ${result.data?.path ?? path}`);
        await refresh();
      } catch (error) {
        toast(error?.message ?? "Recovery failed", "bad");
      }
    });
  }
}

/* ----------------------------------------------------------------- boot */

async function installEventListeners() {
  await listen("devrelay-tray-refresh", () => {
    refresh();
  });
  await listen("devrelay-tray-notice", (event) => {
    const payload = event?.payload ?? {};
    toast(payload.message ?? "Tray action", payload.kind === "bad" ? "bad" : "good");
  });
  await listen("devrelay-tray-open-runs", (event) => {
    const payload = event?.payload ?? {};
    if (payload.project_id) {
      state.selectedProjectId = payload.project_id;
    }
    state.view = "runs";
    const target = payload.target_label ? ` for ${payload.target_label}` : "";
    toast(`Run elsewhere${target} is not wired to the agent yet`, "warn");
    render();
  });
  await listen("devrelay-agent-connected", (event) => {
    markEventBridgeConnected(event?.payload);
    queueEventRefresh(250);
  });
  await listen("devrelay-agent-event", (event) => {
    markEventBridgeEvent(event?.payload ?? {});
    queueEventRefresh(400);
  });
  await listen("devrelay-agent-gap", (event) => {
    markEventBridgeGap(event?.payload);
    queueEventRefresh(0);
  });
  await listen("devrelay-agent-disconnected", (event) => {
    markEventBridgeDisconnected(event?.payload);
    queueEventRefresh(0);
  });
}

document.addEventListener("keydown", handleGlobalKeydown);
installEventListeners().finally(refresh);
