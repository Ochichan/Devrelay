const app = document.querySelector("#app");

const views = [
  ["continue", "Continue", "play"],
  ["projects", "Projects", "folder"],
  ["devices", "Devices", "monitor"],
  ["runs", "Runs", "terminal"],
  ["activity", "Activity", "pulse"],
  ["settings", "Settings", "settings"],
];

const state = {
  view: "continue",
  loading: true,
  operation: null,
  selectedProjectId: null,
  projectFilter: "",
  bootstrap: null,
  projectStatus: new Map(),
  runtimeError: null,
  eventBridge: {
    connected: false,
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
};

const icons = {
  play: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M8 5v14l11-7-11-7Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>',
  folder: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2h6.5A2.5 2.5 0 0 1 21 9.5v7A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5v-9Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>',
  monitor: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 5h16v11H4V5Zm5 15h6m-3-4v4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  terminal: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="m5 7 5 5-5 5m8 0h6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  pulse: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 12h4l2-6 4 12 2-6h4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  settings: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 8.5a3.5 3.5 0 1 1 0 7 3.5 3.5 0 0 1 0-7Z" stroke="currentColor" stroke-width="2"/><path d="M19 12a7.4 7.4 0 0 0-.1-1l2-1.5-2-3.4-2.4 1a7.3 7.3 0 0 0-1.8-1L14.4 3h-4.8l-.3 3.1a7.3 7.3 0 0 0-1.8 1l-2.4-1-2 3.4 2 1.5a7.4 7.4 0 0 0 0 2l-2 1.5 2 3.4 2.4-1a7.3 7.3 0 0 0 1.8 1l.3 3.1h4.8l.3-3.1a7.3 7.3 0 0 0 1.8-1l2.4 1 2-3.4-2-1.5c.1-.3.1-.7.1-1Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>',
  refresh: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M20 12a8 8 0 0 1-13.7 5.7M4 12A8 8 0 0 1 17.7 6.3M18 3v4h-4M6 21v-4h4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  check: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="m5 12 4 4 10-9" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  x: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  box: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="m12 3 8 4.5v9L12 21l-8-4.5v-9L12 3Zm0 9 8-4.5M12 12 4 7.5m8 4.5v9" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>',
  external: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M14 4h6v6M10 14 20 4M20 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  download: '<svg class="icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 3v12m0 0 5-5m-5 5-5-5M5 21h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
};

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

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
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

function methods() {
  return new Set(state.bootstrap?.agent?.methods ?? []);
}

function projects() {
  return state.bootstrap?.projects ?? [];
}

function devices() {
  return state.bootstrap?.devices ?? [];
}

function runs() {
  return state.bootstrap?.runs ?? [];
}

function activity() {
  return state.bootstrap?.activity ?? [];
}

function liveEvents() {
  return state.eventBridge.events ?? [];
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

function selectedProject() {
  const all = projects();
  if (!state.selectedProjectId && all.length > 0) {
    state.selectedProjectId = all[0].project_id;
  }
  return all.find((project) => project.project_id === state.selectedProjectId) ?? all[0] ?? null;
}

function projectStatus(projectId) {
  return state.projectStatus.get(projectId);
}

function activeWorkspace(project) {
  const entries = Object.values(project?.workspaces ?? {});
  const deviceId = state.bootstrap?.settings?.device_id;
  return (
    entries.find((workspace) => workspace.state === "active" && workspace.device_id === deviceId) ??
    entries.find((workspace) => workspace.state === "active") ??
    entries[0] ??
    null
  );
}

function projectLeases(projectId) {
  return leases().filter((lease) => lease.project_id === projectId);
}

function activeLease(projectId) {
  const entries = projectLeases(projectId);
  return (
    entries.find((lease) => lease.state === "active") ??
    entries.find((lease) => lease.state === "handoff-pending") ??
    entries[0] ??
    null
  );
}

function targetDevices() {
  const currentDeviceId = state.bootstrap?.settings?.device_id;
  return devices().filter((device) => device.device_id !== currentDeviceId);
}

function currentDevice() {
  const settings = state.bootstrap?.settings;
  const runtime = state.bootstrap?.runtime;
  const deviceId = settings?.device_id;
  return (
    devices().find((device) => device.device_id === deviceId) ?? {
      device_id: deviceId ?? "local",
      display_name: settings?.device_name ?? "Local device",
      platform_key: runtime?.platform_key ?? "unknown",
      architecture: runtime?.architecture ?? "",
      last_seen_unix_seconds: null,
    }
  );
}

function latestSnapshot(projectId) {
  return snapshots()
    .filter((snapshot) => snapshot.project_id === projectId)
    .sort((left, right) => (right.sequence_number ?? 0) - (left.sequence_number ?? 0))[0];
}

function latestHandoff(projectId) {
  const entries = handoffs().filter((handoff) => handoff.record?.project_id === projectId);
  const active = entries.find(
    (handoff) => !["committed", "aborted"].includes(handoff.record?.state)
  );
  return (
    active ??
    entries.sort((left, right) => {
      const leftTime = left.record?.expires_at_unix_seconds ?? 0;
      const rightTime = right.record?.expires_at_unix_seconds ?? 0;
      return rightTime - leftTime;
    })[0]
  );
}

function incomingHandoff(projectId) {
  const localDeviceId = state.bootstrap?.settings?.device_id;
  if (!localDeviceId) return null;
  return (
    handoffs()
      .filter(
        (handoff) =>
          handoff.record?.project_id === projectId &&
          handoff.record?.target_device_id === localDeviceId &&
          handoffIsOpen(handoff)
      )
      .sort((left, right) => {
        const leftTime = left.record?.expires_at_unix_seconds ?? 0;
        const rightTime = right.record?.expires_at_unix_seconds ?? 0;
        return leftTime - rightTime;
      })[0] ?? null
  );
}

function handoffIsOpen(handoff) {
  return Boolean(handoff && !["committed", "aborted"].includes(handoff.record?.state));
}

function deviceName(deviceId) {
  if (!deviceId) return "No writer recorded";
  const found = devices().find((device) => device.device_id === deviceId);
  return found?.display_name ?? deviceId;
}

function eventDeviceName(deviceId) {
  const found = devices().find((device) => device.device_id === deviceId);
  return found?.display_name ?? deviceId ?? "Unknown device";
}

function activeWriterRow(workspace, lease) {
  if (lease) {
    const tone = lease.state === "active" ? "good" : "warn";
    return `<div class="status-row"><span class="dot ${tone}"></span><div><strong>${escapeHtml(deviceName(lease.holder_device_id))}</strong><span>${escapeHtml(titleize(lease.state))} writer from agent state</span></div><span class="badge ${tone}">Writer</span></div>`;
  }
  return `<div class="status-row"><span class="dot ${workspace?.state === "active" ? "good" : "warn"}"></span><div><strong>${escapeHtml(workspace?.device_id ?? "No writer recorded")}</strong><span>${escapeHtml(workspace?.local_path ?? "No workspace path")}</span></div><span class="badge">${escapeHtml(workspace?.state ?? "unknown")}</span></div>`;
}

function handoffTone(handoff) {
  const state = handoff?.record?.state;
  if (!state) return "warn";
  if (state === "committed") return "good";
  if (state === "aborted") return "bad";
  return "warn";
}

function handoffRow(handoff) {
  if (!handoff) {
    return '<div class="status-row"><span class="dot warn"></span><div><strong>No handoff in progress</strong><span>Waiting for a verified target continuation.</span></div><span class="badge">idle</span></div>';
  }
  const record = handoff.record;
  const tone = handoffTone(handoff);
  const target = record.target_device_id ? `to ${record.target_device_id}` : "target pending";
  const remaining = formatUntil(record.expires_at_unix_seconds);
  const expires = remaining === "expired" ? "expired" : `expires in ${remaining}`;
  const action = handoffIsOpen(handoff)
    ? `<button class="button danger" data-action="handoff-abort" data-project-id="${escapeHtml(record.project_id)}" data-handoff-id="${escapeHtml(record.handoff_id)}" ${state.operation ? "disabled" : ""}>${icons.x}<span>Abort handoff</span></button>`
    : `<span class="badge ${tone}">${escapeHtml(shortId(record.handoff_id))}</span>`;
  return `<div class="status-row"><span class="dot ${tone}"></span><div><strong>${escapeHtml(titleize(record.state))}</strong><span>${escapeHtml(target)} - ${escapeHtml(expires)}</span></div>${action}</div>`;
}

function recentlySeen(device) {
  return Math.floor(Date.now() / 1000) - (device?.last_seen_unix_seconds ?? 0) < 300;
}

function macLinuxTarget(device) {
  const platform = String(device?.platform_key ?? "");
  return (
    ["darwin", "macos", "linux", "linux-gnu"].includes(platform) ||
    platform.startsWith("darwin-") ||
    platform.startsWith("linux-gnu-")
  );
}

function targetReadiness(device, context) {
  const { handoffReady, lease, openHandoff } = context;
  if (!macLinuxTarget(device)) {
    return {
      ready: false,
      tone: "warn",
      label: "Later OS",
      detail: "Windows and WSL UI wait for named pipe IPC hardening.",
    };
  }
  if (!recentlySeen(device)) {
    return {
      ready: false,
      tone: "warn",
      label: "Offline",
      detail: `Last seen ${formatAge(device.last_seen_unix_seconds)}`,
    };
  }
  if (!handoffReady) {
    return {
      ready: false,
      tone: "warn",
      label: "RPC missing",
      detail: "This agent build does not expose handoff.begin yet.",
    };
  }
  if (openHandoff) {
    return {
      ready: false,
      tone: "warn",
      label: "Preparing",
      detail: "A handoff is already waiting for target apply.",
    };
  }
  if (!lease || lease.state !== "active") {
    return {
      ready: false,
      tone: "warn",
      label: "No writer",
      detail: "This device does not hold an active writer lease.",
    };
  }
  if (lease.holder_device_id !== state.bootstrap?.settings?.device_id) {
    return {
      ready: false,
      tone: "warn",
      label: "Not writer",
      detail: "Only the active writer can prepare a handoff.",
    };
  }
  return {
    ready: true,
    tone: "good",
    label: "Ready",
    detail: "Fresh checkpoint and target preparation can start.",
  };
}

function projectTargetAvailability(project) {
  const targets = targetDevices();
  const lease = activeLease(project.project_id);
  const openHandoff = handoffIsOpen(latestHandoff(project.project_id));
  const handoffReady = methods().has("handoff.begin");
  const entries = targets.map((device) => ({
    device,
    readiness: targetReadiness(device, { handoffReady, lease, openHandoff }),
  }));
  const readyCount = entries.filter((entry) => entry.readiness.ready).length;
  const label = targets.length === 0 ? "No targets" : `${readyCount}/${targets.length} ready`;
  const detail =
    entries.length === 0
      ? "Pair another device before starting a desktop handoff."
      : entries.map((entry) => `${entry.device.display_name}: ${entry.readiness.label}`).join("; ");
  return {
    entries,
    readyCount,
    total: targets.length,
    tone: readyCount > 0 ? "good" : "warn",
    label,
    detail,
  };
}

function projectSession(project) {
  const workspace = activeWorkspace(project);
  const checkpoint = latestSnapshot(project.project_id);
  return {
    workspace,
    label: workspace?.workspace_id ?? checkpoint?.session_id ?? "No session recorded",
    detail: workspace?.local_path ?? project.local_path,
    state: workspace?.state ?? "unknown",
  };
}

function projectWriter(project) {
  const lease = activeLease(project.project_id);
  const workspace = activeWorkspace(project);
  const workspaceWriter = workspace?.state === "active" ? workspace.device_id : null;
  const writerId = lease?.holder_device_id ?? workspaceWriter;
  return {
    label: deviceName(writerId),
    detail: lease ? `${titleize(lease.state)} writer` : workspaceWriter ? "Active workspace" : "No active writer",
    tone: lease?.state === "active" ? "good" : "warn",
  };
}

function projectCheckpoint(project) {
  const checkpoint = latestSnapshot(project.project_id);
  if (!checkpoint) {
    return {
      label: "No checkpoint",
      detail: "Create a checkpoint before handoff.",
      tone: "warn",
    };
  }
  return {
    label: shortId(checkpoint.snapshot_id),
    detail: `${formatAge(checkpoint.created_at_unix_seconds)} - ${checkpoint.label ?? "unlabeled"}`,
    tone: "good",
  };
}

function projectAttention(project) {
  const status = projectStatus(project.project_id);
  const counts = statusCounts(status?.data);
  const availability = projectTargetAvailability(project);
  if (status?.error) {
    return { needsAttention: true, tone: "bad", label: "Status error", detail: status.error };
  }
  if (counts.unmerged > 0) {
    return {
      needsAttention: true,
      tone: "bad",
      label: "Conflicts",
      detail: "Resolve conflicts before handoff.",
    };
  }
  if (handoffIsOpen(latestHandoff(project.project_id))) {
    return {
      needsAttention: true,
      tone: "warn",
      label: "Handoff open",
      detail: "Finish or abort the active handoff.",
    };
  }
  if (!latestSnapshot(project.project_id)) {
    return {
      needsAttention: true,
      tone: "warn",
      label: "No checkpoint",
      detail: "Checkpoint status is empty.",
    };
  }
  const lease = activeLease(project.project_id);
  if (!lease || lease.state !== "active") {
    return {
      needsAttention: true,
      tone: "warn",
      label: "No active writer",
      detail: "Writer state is not active.",
    };
  }
  if (availability.total > 0 && availability.readyCount === 0) {
    return {
      needsAttention: true,
      tone: "warn",
      label: "No ready target",
      detail: availability.detail,
    };
  }
  return { needsAttention: false, tone: "good", label: "Ready", detail: "No immediate action needed." };
}

function projectSearchText(project) {
  const session = projectSession(project);
  const writer = projectWriter(project);
  return [
    project.display_name,
    project.local_path,
    project.project_id,
    session.label,
    session.detail,
    writer.label,
    writer.detail,
  ]
    .join(" ")
    .toLowerCase();
}

function filteredProjects() {
  const query = state.projectFilter.trim().toLowerCase();
  if (!query) return projects();
  return projects().filter((project) => projectSearchText(project).includes(query));
}

function continueHereReadiness(handoff) {
  if (!handoff) {
    return {
      ready: false,
      tone: "warn",
      label: "No incoming handoff",
      detail: "Start a handoff from another device before continuing here.",
    };
  }
  const record = handoff.record ?? {};
  const remaining = formatUntil(record.expires_at_unix_seconds);
  if (remaining === "expired") {
    return {
      ready: false,
      tone: "bad",
      label: "Expired",
      detail: "Abort this handoff and start again from the source device.",
    };
  }
  const requiredMethods = [
    "apply.snapshot",
    "handoff.target.verify",
    "handoff.source.ready",
    "handoff.commit",
  ];
  const missingMethod = requiredMethods.find((method) => !methods().has(method));
  if (missingMethod) {
    return {
      ready: false,
      tone: "warn",
      label: "RPC missing",
      detail: `${missingMethod} is not exposed by this agent build.`,
    };
  }
  return {
    ready: true,
    tone: "good",
    label: titleize(record.state),
    detail: `Ready to apply and verify this handoff on ${deviceName(record.target_device_id)}.`,
  };
}

function continueHereRow(handoff, readiness) {
  const target = handoff?.record?.target_device_id
    ? deviceName(handoff.record.target_device_id)
    : "This device";
  return `<div class="status-row"><span class="dot ${readiness.tone}"></span><div><strong>${escapeHtml(readiness.label)}</strong><span>${escapeHtml(readiness.detail)}</span></div><span class="badge ${readiness.tone}">${escapeHtml(target)}</span></div>`;
}

function statusCounts(statusResult) {
  return statusResult?.status?.counts ?? {
    staged: 0,
    unstaged: 0,
    untracked: 0,
    ignored: 0,
    unmerged: 0,
  };
}

function statusBadge(statusResult, loading, error) {
  if (loading) return '<span class="badge">Loading</span>';
  if (error) return '<span class="badge bad">Status error</span>';
  if (!statusResult) return '<span class="badge">Not loaded</span>';
  return statusResult.status?.clean
    ? '<span class="badge good">Clean</span>'
    : '<span class="badge warn">Local changes</span>';
}

function sequenceLabel(value) {
  return value === null || value === undefined ? "none" : `#${value}`;
}

function eventBridgeStatus() {
  const bridge = state.eventBridge;
  if (bridge.stale) return { tone: "warn", label: "Event gap", detail: "Refreshing from agent state" };
  if (bridge.connected) {
    return {
      tone: "good",
      label: bridge.refreshing ? "Events syncing" : "Events live",
      detail: bridge.lastEvent
        ? `${bridge.lastEvent.type} ${sequenceLabel(bridge.lastEvent.sequence)}`
        : `Subscribed at ${formatClock(bridge.lastConnectedAt)}`,
    };
  }
  if (bridge.lastError) {
    return { tone: "bad", label: "Events reconnecting", detail: bridge.lastError };
  }
  return { tone: "warn", label: "Events connecting", detail: "Waiting for the local agent stream" };
}

function currentTitle() {
  const view = views.find(([id]) => id === state.view);
  if (!view) return "DevRelay";
  if (state.view === "continue") {
    const project = selectedProject();
    return project ? `Continue ${project.display_name}` : "Continue";
  }
  return view[1];
}

function toast(message, kind = "good") {
  const id = crypto.randomUUID?.() ?? String(Date.now());
  state.toasts.push({ id, message, kind });
  render();
  window.setTimeout(() => {
    state.toasts = state.toasts.filter((item) => item.id !== id);
    render();
  }, 5000);
}

async function refresh() {
  state.loading = true;
  state.runtimeError = null;
  render();
  try {
    const bootstrap = await invoke("ui_bootstrap");
    state.bootstrap = bootstrap;
    if (!state.selectedProjectId || !projects().some((project) => project.project_id === state.selectedProjectId)) {
      state.selectedProjectId = projects()[0]?.project_id ?? null;
    }
    state.loading = false;
    render();
    const project = selectedProject();
    if (project) await refreshProjectStatus(project.project_id, false);
    return true;
  } catch (error) {
    state.bootstrap = null;
    state.runtimeError = String(error?.message ?? error);
    state.loading = false;
    render();
    return false;
  }
}

async function refreshProjectStatus(projectId, announce = true) {
  if (!projectId) return;
  state.projectStatus.set(projectId, { loading: true, error: null, data: null });
  render();
  try {
    const result = await invoke("project_status", { projectId });
    if (!result.ok) throw new Error(result.message);
    state.projectStatus.set(projectId, { loading: false, error: null, data: result.data });
    if (announce) toast("Project status loaded");
  } catch (error) {
    state.projectStatus.set(projectId, {
      loading: false,
      error: String(error?.message ?? error),
      data: null,
    });
    if (announce) toast("Project status failed", "bad");
  }
  render();
}

async function runOperation(label, fn) {
  state.operation = label;
  render();
  try {
    await fn();
  } finally {
    state.operation = null;
    render();
  }
}

function shell() {
  const bootstrap = state.bootstrap;
  const agent = bootstrap?.agent;
  const settings = bootstrap?.settings;
  const connected = Boolean(agent?.connected);
  const eventStatus = eventBridgeStatus();
  const projectCount = projects().length;
  return `
    <div class="app-shell">
      <aside class="sidebar">
        <div class="brand">
          <div class="brand-row">
            <div class="brand-symbol">${icons.box}</div>
            <div class="brand-title">
              <h1>DevRelay</h1>
              <p>${escapeHtml(settings?.fabric_name ?? "Local fabric")}</p>
            </div>
          </div>
          <div class="brand-status">
            <div class="agent-pill">
              <span class="dot ${connected ? "good" : state.runtimeError ? "bad" : "warn"}"></span>
              <span>${connected ? "Agent connected" : state.runtimeError ? "Runtime unavailable" : "Agent unavailable"}</span>
            </div>
            <div class="agent-pill" title="${escapeHtml(eventStatus.detail)}">
              <span class="dot ${eventStatus.tone}"></span>
              <span>${escapeHtml(eventStatus.label)}</span>
            </div>
          </div>
        </div>
        <div class="sidebar-scroll" data-scroll-container>
          <nav class="nav-section" aria-label="Main">
            <div class="nav-label">Views</div>
            ${views
              .map(([id, label, icon]) => {
                const count =
                  id === "projects"
                    ? projectCount
                    : id === "devices"
                      ? devices().length
                      : id === "runs"
                        ? runs().length
                        : id === "activity"
                          ? activity().length
                          : "";
                return `<button class="nav-button" data-view="${id}" aria-current="${state.view === id ? "page" : "false"}" title="${escapeHtml(label)}">
                  ${icons[icon]}<span>${escapeHtml(label)}</span>${count === "" ? "" : `<span class="nav-count">${count}</span>`}
                </button>`;
              })
              .join("")}
          </nav>
          <div class="nav-section">
            <div class="nav-label">Projects</div>
            ${
              projectCount === 0
                ? '<div class="empty"><strong>No projects</strong><p>Register a project from the CLI to see it here.</p></div>'
                : projects()
                    .map(
                      (project) => `<button class="project-button" data-project="${escapeHtml(project.project_id)}" aria-current="${project.project_id === selectedProject()?.project_id}">
                        <span>${escapeHtml(project.display_name)}</span>
                        <span class="nav-count">${Object.keys(project.workspaces ?? {}).length}</span>
                      </button>`
                    )
                    .join("")
            }
          </div>
        </div>
        <div class="sidebar-foot">
          <span>${escapeHtml(settings?.device_name ?? bootstrap?.runtime?.platform_key ?? "local device")}</span>
          <code>${escapeHtml(bootstrap?.runtime?.devrelay_home ?? "runtime not loaded")}</code>
        </div>
      </aside>
      <section class="workspace">
        <header class="topbar">
          <div class="title-group">
            <p>${escapeHtml(bootstrap?.runtime?.agent_socket_path ?? "Desktop runtime")}</p>
            <h2>${escapeHtml(currentTitle())}</h2>
          </div>
          <div class="top-actions">
            <button class="button icon-only" data-action="refresh" title="Refresh" aria-label="Refresh">${icons.refresh}</button>
            <button class="button" data-action="diagnostics" ${state.operation ? "disabled" : ""} title="Export diagnostics">${icons.download}<span>Diagnostics</span></button>
          </div>
        </header>
        <main class="main-scroll" data-scroll-container>
          ${state.loading ? loadingScreen() : state.runtimeError ? runtimeErrorScreen() : renderView()}
        </main>
      </section>
    </div>
    ${renderToasts()}
  `;
}

function loadingScreen() {
  return `<section class="screen"><div class="loading-line"><span class="small-spinner"></span><span>Loading runtime state</span></div></section>`;
}

function runtimeErrorScreen() {
  return `
    <section class="screen">
      <div class="panel">
        <div class="panel-head"><div><h3>Desktop runtime unavailable</h3><p>The app did not receive a Tauri command bridge.</p></div></div>
        <div class="panel-body">
          <div class="error-box">${escapeHtml(state.runtimeError)}</div>
        </div>
      </div>
    </section>
  `;
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

function renderContinue() {
  const project = selectedProject();
  if (!project) {
    return `
      <section class="screen">
        ${agentErrors()}
        <div class="empty"><strong>No registered projects</strong><p>Add a project with the local CLI before continuing work from the desktop app.</p></div>
      </section>
    `;
  }
  const status = projectStatus(project.project_id);
  const counts = statusCounts(status?.data);
  const workspace = activeWorkspace(project);
  const lease = activeLease(project.project_id);
  const device = currentDevice();
  const latest = activity().find((event) => event.project_id === project.project_id);
  const checkpoint = latestSnapshot(project.project_id);
  const handoff = latestHandoff(project.project_id);
  const incoming = incomingHandoff(project.project_id);
  const continueReadiness = continueHereReadiness(incoming);
  const openHandoff = handoffIsOpen(handoff);
  const availableTargets = targetDevices();
  const handoffReady = methods().has("handoff.begin");
  const suggestedSession = workspace?.workspace_id ?? checkpoint?.session_id ?? project.project_id;
  const handoffPanelCopy = openHandoff
    ? "Target preparation is in progress; continue on the target device to apply and verify"
    : handoffReady
      ? "Start target preparation with a fresh checkpoint; target apply and verification remain pending"
      : "Handoff RPC is not exposed by this agent build";
  const continueDisabled = !continueReadiness.ready || Boolean(state.operation);
  return `
    <section class="screen">
      ${agentErrors()}
      <div class="screen-grid">
        <div class="panel">
          <div class="panel-body project-hero">
            <div class="project-title">
              <div>
                <h3>${escapeHtml(project.display_name)}</h3>
                <p>${escapeHtml(project.local_path)}</p>
              </div>
              ${statusBadge(status?.data, status?.loading, status?.error)}
            </div>
            ${status?.error ? `<div class="error-box">${escapeHtml(status.error)}</div>` : ""}
            <div class="summary-grid">
              <div class="metric"><strong>${counts.staged}</strong><span>Staged</span></div>
              <div class="metric"><strong>${counts.unstaged}</strong><span>Modified</span></div>
              <div class="metric"><strong>${counts.untracked}</strong><span>Untracked</span></div>
              <div class="metric"><strong>${counts.unmerged}</strong><span>Conflicts</span></div>
            </div>
            <div class="button-row">
              <button class="button primary" data-action="handoff-continue-here" data-project-id="${escapeHtml(project.project_id)}" data-handoff-id="${escapeHtml(incoming?.record?.handoff_id ?? "")}" ${continueDisabled ? "disabled" : ""}>${icons.play}<span>Continue here</span></button>
              <button class="button" data-action="checkpoint" data-project-id="${escapeHtml(project.project_id)}" ${state.operation ? "disabled" : ""}>${icons.check}<span>Checkpoint now</span></button>
              <button class="button" data-action="project-status" data-project-id="${escapeHtml(project.project_id)}" ${status?.loading ? "disabled" : ""}>${icons.refresh}<span>Status</span></button>
              <button class="button" data-action="open-project" data-project-id="${escapeHtml(project.project_id)}">${icons.external}<span>Open folder</span></button>
            </div>
          </div>
        </div>
        <div class="panel flat">
          <div class="panel-head"><div><h3>Continuation state</h3><p>${escapeHtml(suggestedSession)}</p></div></div>
          <div class="panel-body status-stack">
            <div class="status-row"><span class="dot good"></span><div><strong>${escapeHtml(device.display_name ?? "Local device")}</strong><span>${escapeHtml(device.platform_key ?? "unknown")} ${escapeHtml(device.architecture ?? "")}</span></div><span class="badge good">This device</span></div>
            <div class="status-row"><span class="dot ${suggestedSession ? "good" : "warn"}"></span><div><strong>Suggested session</strong><span>${escapeHtml(suggestedSession ?? "No continuation session recorded")}</span></div><span class="badge">${workspace?.state ? escapeHtml(workspace.state) : "selected"}</span></div>
            ${activeWriterRow(workspace, lease)}
            ${handoffRow(handoff)}
            ${continueHereRow(incoming, continueReadiness)}
            <div class="status-row"><span class="dot ${checkpoint ? "good" : "warn"}"></span><div><strong>${checkpoint ? escapeHtml(shortId(checkpoint.snapshot_id)) : "No checkpoint recorded"}</strong><span>${checkpoint ? `${formatAge(checkpoint.created_at_unix_seconds)} - ${escapeHtml(checkpoint.label ?? "unlabeled")}` : "Create a checkpoint before cross-device handoff."}</span></div><span class="badge">${checkpoint ? `#${checkpoint.sequence_number}` : "empty"}</span></div>
            <div class="status-row"><span class="dot ${latest ? "good" : "warn"}"></span><div><strong>${latest ? escapeHtml(latest.summary) : "No activity recorded"}</strong><span>${latest ? formatAge(latest.created_at_unix_seconds) : "waiting for agent events"}</span></div><span class="badge">${latest ? escapeHtml(latest.outcome) : "empty"}</span></div>
          </div>
        </div>
      </div>
      <div class="panel">
        <div class="panel-head">
          <div><h3>Continue on another device</h3><p>${handoffPanelCopy}</p></div>
        </div>
        <div class="panel-body">
          ${
            availableTargets.length === 0
              ? '<div class="empty"><strong>No paired target devices</strong><p>Pair another device before starting a desktop handoff.</p></div>'
              : `<div class="list">${availableTargets
                  .map((device) => {
                    const readiness = targetReadiness(device, { handoffReady, lease, openHandoff });
                    const disabled = !readiness.ready || Boolean(state.operation);
                    return `<div class="list-item">
                      <div class="list-item-row">
                        <div><strong>${escapeHtml(device.display_name)}</strong><span>${escapeHtml(device.platform_key)} ${escapeHtml(device.architecture)} - ${escapeHtml(readiness.detail)}</span></div>
                        <span class="badge ${readiness.tone}">${escapeHtml(readiness.label)}</span>
                      </div>
                      <button class="button ${readiness.ready ? "primary" : ""}" data-action="handoff-prepare" data-project-id="${escapeHtml(project.project_id)}" data-target-device-id="${escapeHtml(device.device_id)}" ${disabled ? "disabled" : ""}>${icons.play}<span>${readiness.ready ? "Prepare handoff" : readiness.label}</span></button>
                    </div>`;
                  })
                  .join("")}</div>`
          }
        </div>
      </div>
    </section>
  `;
}

function renderProjects() {
  const visibleProjects = filteredProjects();
  const decorated = visibleProjects.map((project) => ({
    project,
    attention: projectAttention(project),
  }));
  const needsAttention = decorated.filter((entry) => entry.attention.needsAttention);
  const ready = decorated.filter((entry) => !entry.attention.needsAttention);
  const rowForProject = ({ project, attention }) => {
    const status = projectStatus(project.project_id);
    const counts = statusCounts(status?.data);
    const session = projectSession(project);
    const writer = projectWriter(project);
    const checkpoint = projectCheckpoint(project);
    const availability = projectTargetAvailability(project);
    return `<tr>
      <td><div class="cell-main"><strong>${escapeHtml(project.display_name)}</strong><span>${escapeHtml(project.local_path)}</span></div></td>
      <td><div class="cell-main"><strong>${escapeHtml(session.label)}</strong><span>${escapeHtml(session.state)} - ${escapeHtml(session.detail)}</span></div></td>
      <td><div class="cell-main"><strong>${escapeHtml(writer.label)}</strong><span>${escapeHtml(writer.detail)}</span></div></td>
      <td><div class="cell-main"><strong>${escapeHtml(checkpoint.label)}</strong><span>${escapeHtml(checkpoint.detail)}</span></div></td>
      <td><div class="cell-main"><strong><span class="badge ${availability.tone}">${escapeHtml(availability.label)}</span></strong><span>${escapeHtml(availability.detail)}</span></div></td>
      <td><div class="cell-main"><strong><span class="badge ${attention.tone}">${escapeHtml(attention.label)}</span></strong><span>${escapeHtml(attention.detail)}</span><span>${counts.staged} staged, ${counts.unstaged} modified, ${counts.untracked} untracked</span></div></td>
      <td>
        <div class="button-row">
          <button class="button" data-action="select-project" data-project-id="${escapeHtml(project.project_id)}">${icons.play}<span>Details</span></button>
          <button class="button icon-only" data-action="project-status" data-project-id="${escapeHtml(project.project_id)}" title="Status" aria-label="Status">${icons.refresh}</button>
        </div>
      </td>
    </tr>`;
  };
  const groupRows = (label, entries) =>
    entries.length === 0
      ? ""
      : `<tr class="table-group-row"><td colspan="7">${escapeHtml(label)} (${entries.length})</td></tr>${entries.map(rowForProject).join("")}`;
  const rows = `${groupRows("Needs attention", needsAttention)}${groupRows("Ready", ready)}`;
  return `
    <section class="screen">
      ${agentErrors()}
      <div class="panel">
        <div class="panel-head">
          <div><h3>Projects</h3><p>${visibleProjects.length} of ${projects().length} registered - ${needsAttention.length} need attention</p></div>
          <div class="filter-field"><label class="visually-hidden" for="project_filter">Filter projects</label><input id="project_filter" data-project-filter value="${escapeHtml(state.projectFilter)}" placeholder="Filter projects" aria-label="Filter projects" /></div>
        </div>
        <div class="panel-body">
          ${
            projects().length === 0
              ? '<div class="empty"><strong>No projects</strong><p>Use the local CLI to register a repository.</p></div>'
              : visibleProjects.length === 0
                ? '<div class="empty"><strong>No matching projects</strong><p>Clear the filter to show all registered projects.</p></div>'
                : `<div class="table-scroll" data-scroll-container><table class="projects-table"><thead><tr><th>Project</th><th>Active session</th><th>Writer</th><th>Checkpoint</th><th>Target availability</th><th>Needs attention</th><th>Actions</th></tr></thead><tbody>${rows}</tbody></table></div>`
          }
        </div>
      </div>
    </section>
  `;
}

function renderDevices() {
  const currentDeviceId = state.bootstrap?.settings?.device_id;
  const rows = devices()
    .map((device) => `<tr>
      <td><div class="cell-main"><strong>${escapeHtml(device.display_name)}</strong><span>${escapeHtml(device.device_id)}</span></div></td>
      <td>${escapeHtml(device.platform_key)}</td>
      <td>${escapeHtml(device.architecture)}</td>
      <td>${formatAge(device.last_seen_unix_seconds)}</td>
      <td>${device.device_id === currentDeviceId ? '<span class="badge good">This device</span>' : '<span class="badge">Paired</span>'}</td>
    </tr>`)
    .join("");
  return `
    <section class="screen">
      ${agentErrors()}
      <div class="panel">
        <div class="panel-head"><div><h3>Devices</h3><p>${devices().length} known identities</p></div></div>
        <div class="panel-body">
          ${
            devices().length === 0
              ? '<div class="empty"><strong>No paired devices</strong><p>Pairing records will appear after the agent writes device metadata.</p></div>'
              : `<div class="table-scroll" data-scroll-container><table><thead><tr><th>Device</th><th>Platform</th><th>Arch</th><th>Last seen</th><th>Role</th></tr></thead><tbody>${rows}</tbody></table></div>`
          }
        </div>
      </div>
    </section>
  `;
}

function renderRuns() {
  const rows = runs()
    .map((run) => `<tr>
      <td><div class="cell-main"><strong>${escapeHtml(shortId(run.task_run_id))}</strong><span>${escapeHtml(run.project_id)}</span></div></td>
      <td><span class="badge">${escapeHtml(run.state)}</span></td>
      <td><code>${escapeHtml(run.command ?? "-")}</code></td>
      <td>${formatAge(run.updated_at_unix_seconds)}</td>
      <td><pre class="json-box">${escapeHtml(JSON.stringify(run.metadata ?? {}, null, 2))}</pre></td>
    </tr>`)
    .join("");
  return `
    <section class="screen">
      ${agentErrors()}
      <div class="panel">
        <div class="panel-head"><div><h3>Runs</h3><p>${runs().length} task records</p></div></div>
        <div class="panel-body">
          ${
            runs().length === 0
              ? '<div class="empty"><strong>No task runs</strong><p>Remote and local task records will appear here after execution.</p></div>'
              : `<div class="table-scroll" data-scroll-container><table><thead><tr><th>Run</th><th>State</th><th>Command</th><th>Updated</th><th>Metadata</th></tr></thead><tbody>${rows}</tbody></table></div>`
          }
        </div>
      </div>
    </section>
  `;
}

function renderActivity() {
  const handoffEvents = liveEvents().filter((event) => event.type === "handoff.state.changed");
  const handoffRows = handoffEvents
    .map((event) => {
      const payload = event.payload ?? {};
      const stateLabel = titleize(payload.state);
      const previous = payload.previous_state ? `from ${titleize(payload.previous_state)}` : "started";
      const target = eventDeviceName(payload.target_device_id);
      return `<div class="list-item">
        <div class="list-item-row">
          <div><strong>${escapeHtml(stateLabel)}</strong><span>${escapeHtml(previous)} - ${escapeHtml(target)} - ${formatClock(event.occurredAt ?? event.receivedAt)}</span></div>
          <span class="badge ${payload.state === "committed" ? "good" : payload.state === "aborted" ? "bad" : "warn"}">${escapeHtml(event.type)}</span>
        </div>
      </div>`;
    })
    .join("");
  const rows = activity()
    .map((event) => `<div class="list-item">
      <div class="list-item-row">
        <div><strong>${escapeHtml(event.summary)}</strong><span>${escapeHtml(event.type)} - ${escapeHtml(event.project_id ?? "global")} - ${formatAge(event.created_at_unix_seconds)}</span></div>
        <span class="badge ${event.outcome === "succeeded" ? "good" : event.outcome === "failed" ? "bad" : ""}">${escapeHtml(event.outcome)}</span>
      </div>
      <pre class="json-box">${escapeHtml(JSON.stringify(event.detail ?? {}, null, 2))}</pre>
    </div>`)
    .join("");
  return `
    <section class="screen">
      ${agentErrors()}
      <div class="panel">
        <div class="panel-head"><div><h3>Handoff events</h3><p>${handoffEvents.length} from agent stream</p></div></div>
        <div class="panel-body scroll" data-scroll-container>
          ${handoffEvents.length === 0 ? '<div class="empty"><strong>No handoff events</strong><p>Handoff state changes will appear here after the event stream receives them.</p></div>' : `<div class="list">${handoffRows}</div>`}
        </div>
      </div>
      <div class="panel">
        <div class="panel-head"><div><h3>Activity</h3><p>${activity().length} audit events</p></div></div>
        <div class="panel-body scroll" data-scroll-container>
          ${activity().length === 0 ? '<div class="empty"><strong>No activity</strong><p>Agent audit events will appear here.</p></div>' : `<div class="list">${rows}</div>`}
        </div>
      </div>
    </section>
  `;
}

function renderSettings() {
  const settings = state.bootstrap?.settings;
  if (!settings) {
    return `<section class="screen">${agentErrors()}<div class="empty"><strong>Settings unavailable</strong><p>The agent did not return local settings.</p></div></section>`;
  }
  return `
    <section class="screen">
      ${agentErrors()}
      <div class="screen-grid">
        <form class="panel" data-settings-form>
          <div class="panel-head"><div><h3>Local Settings</h3><p>${escapeHtml(settings.device_id)}</p></div></div>
          <div class="panel-body form-grid">
            <div class="field"><label for="resource_profile">Resource profile</label><select id="resource_profile" name="resource_profile">
              ${["adaptive", "instant", "eco", "custom", "balanced", "performance"].map((profile) => `<option value="${profile}" ${profile === settings.resource_profile ? "selected" : ""}>${titleize(profile)}</option>`).join("")}
            </select></div>
            <div class="field"><label for="editor_command">Editor command</label><input id="editor_command" name="editor_command" value="${escapeHtml(settings.editor_command)}" /></div>
            <label class="check-field"><input type="checkbox" name="mdns_enabled" ${settings.mdns_enabled ? "checked" : ""} /> <span>mDNS discovery</span></label>
            <div class="button-row"><button class="button primary" type="submit" ${state.operation ? "disabled" : ""}>${icons.check}<span>Save</span></button></div>
          </div>
        </form>
        <div class="panel flat">
          <div class="panel-head"><div><h3>Runtime</h3><p>${escapeHtml(state.bootstrap?.runtime?.platform_key)} ${escapeHtml(state.bootstrap?.runtime?.architecture)}</p></div></div>
          <div class="panel-body status-stack">
            <div class="status-row"><span class="dot ${state.bootstrap?.runtime?.agent_socket_exists ? "good" : "bad"}"></span><div><strong>Agent socket</strong><span>${escapeHtml(state.bootstrap?.runtime?.agent_socket_path)}</span></div><span class="badge">${state.bootstrap?.runtime?.agent_socket_exists ? "Found" : "Missing"}</span></div>
            ${renderEventBridgeRow()}
            <div class="status-row"><span class="dot good"></span><div><strong>Anchor mode</strong><span>${escapeHtml(settings.anchor_mode)}</span></div><span class="badge">${escapeHtml(settings.resource_profile)}</span></div>
            <div class="status-row"><span class="dot good"></span><div><strong>Projects</strong><span>${settings.project_count}</span></div><span class="badge">${escapeHtml(settings.device_name)}</span></div>
          </div>
        </div>
      </div>
    </section>
  `;
}

function agentErrors() {
  const bridge = state.eventBridge;
  const notices = [];
  if (bridge.stale && bridge.lastGap) {
    notices.push(
      `<div class="warning-box"><strong>Event stream gap</strong><span>Expected after ${escapeHtml(sequenceLabel(bridge.lastGap.expected_after))}, received ${escapeHtml(sequenceLabel(bridge.lastGap.actual_next))}. The UI is refreshing from the agent snapshot.</span></div>`
    );
  } else if (!bridge.connected && bridge.lastError) {
    notices.push(
      `<div class="warning-box"><strong>Event stream reconnecting</strong><span>${escapeHtml(bridge.lastError)}</span></div>`
    );
  }
  const errors = state.bootstrap?.agent?.errors ?? [];
  if (errors.length > 0) {
    notices.push(`<div class="error-box" data-scroll-container>${errors.map(escapeHtml).join("<br>")}</div>`);
  }
  return notices.join("");
}

function renderEventBridgeRow() {
  const bridge = state.eventBridge;
  const status = eventBridgeStatus();
  const lastEvent = bridge.lastEvent
    ? `${bridge.lastEvent.type} ${sequenceLabel(bridge.lastEvent.sequence)} at ${formatClock(bridge.lastEvent.receivedAt)}`
    : "No event received in this app session";
  const subscription = bridge.subscription
    ? `cursor ${sequenceLabel(bridge.subscription.cursorSequence)}, current ${sequenceLabel(bridge.subscription.currentSequence)}, replayed ${bridge.subscription.replayed}`
    : "Subscription pending";
  return `<div class="status-row"><span class="dot ${status.tone}"></span><div><strong>Event stream</strong><span>${escapeHtml(lastEvent)}</span><span>${escapeHtml(subscription)}</span></div><span class="badge ${status.tone}">${escapeHtml(status.label)}</span></div>`;
}

function renderToasts() {
  if (state.toasts.length === 0) return "";
  return `<div class="toast-region" data-scroll-container>${state.toasts
    .map((item) => `<div class="toast ${escapeHtml(item.kind)}">${escapeHtml(item.message)}</div>`)
    .join("")}</div>`;
}

function render() {
  app.innerHTML = shell();
  attachHandlers();
}

function attachHandlers() {
  app.querySelectorAll("[data-view]").forEach((button) => {
    button.addEventListener("click", () => {
      state.view = button.dataset.view;
      render();
    });
  });
  app.querySelectorAll("[data-project]").forEach((button) => {
    button.addEventListener("click", async () => {
      state.selectedProjectId = button.dataset.project;
      state.view = "continue";
      render();
      await refreshProjectStatus(state.selectedProjectId, false);
    });
  });
  app.querySelectorAll("[data-action]").forEach((button) => {
    button.addEventListener("click", () => handleAction(button));
  });
  const settingsForm = app.querySelector("[data-settings-form]");
  if (settingsForm) {
    settingsForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      const data = new FormData(settingsForm);
      await runOperation("Saving settings", async () => {
        const result = await invoke("settings_update", {
          params: {
            resource_profile: data.get("resource_profile"),
            mdns_enabled: data.get("mdns_enabled") === "on",
            editor_command: data.get("editor_command"),
          },
        });
        if (!result.ok) throw new Error(result.message);
        toast("Settings saved");
        await refresh();
      }).catch((error) => toast(String(error?.message ?? error), "bad"));
    });
  }
  const projectFilter = app.querySelector("[data-project-filter]");
  if (projectFilter) {
    projectFilter.addEventListener("input", () => {
      state.projectFilter = projectFilter.value;
      render();
      const nextFilter = app.querySelector("[data-project-filter]");
      if (nextFilter) {
        nextFilter.focus?.();
        nextFilter.setSelectionRange?.(nextFilter.value.length, nextFilter.value.length);
      }
    });
  }
}

async function handleAction(button) {
  const action = button.dataset.action;
  const projectId = button.dataset.projectId;
  const targetDeviceId = button.dataset.targetDeviceId;
  const handoffId = button.dataset.handoffId;
  if (action === "refresh") {
    await refresh();
    return;
  }
  if (action === "project-status") {
    await refreshProjectStatus(projectId);
    return;
  }
  if (action === "select-project") {
    state.selectedProjectId = projectId;
    state.view = "continue";
    render();
    await refreshProjectStatus(projectId, false);
    return;
  }
  if (action === "checkpoint") {
    await runOperation("Creating checkpoint", async () => {
      const result = await invoke("checkpoint_create", { projectId });
      if (!result.ok) throw new Error(result.message);
      toast("Checkpoint created");
      await refresh();
    }).catch((error) => toast(String(error?.message ?? error), "bad"));
    return;
  }
  if (action === "handoff-prepare") {
    await runOperation("Preparing handoff", async () => {
      const result = await invoke("handoff_prepare", { projectId, targetDeviceId });
      if (!result.ok) throw new Error(result.message);
      toast("Handoff preparation started");
      await refresh();
    }).catch((error) => toast(String(error?.message ?? error), "bad"));
    return;
  }
  if (action === "handoff-continue-here") {
    await runOperation("Continuing here", async () => {
      const result = await invoke("handoff_continue_here", { projectId, handoffId });
      if (!result.ok) throw new Error(result.message);
      toast("Continuation verified");
      await refresh();
    }).catch((error) => toast(String(error?.message ?? error), "bad"));
    return;
  }
  if (action === "handoff-abort") {
    await runOperation("Aborting handoff", async () => {
      const result = await invoke("handoff_abort", { projectId, handoffId });
      if (!result.ok) throw new Error(result.message);
      toast("Handoff aborted");
      await refresh();
    }).catch((error) => toast(String(error?.message ?? error), "bad"));
    return;
  }
  if (action === "open-project") {
    await runOperation("Opening project", async () => {
      const result = await invoke("open_project", { projectId });
      if (!result.ok) throw new Error(result.message);
      toast(`Opened ${result.data}`);
    }).catch((error) => toast(String(error?.message ?? error), "bad"));
    return;
  }
  if (action === "diagnostics") {
    await runOperation("Exporting diagnostics", async () => {
      const result = await invoke("diagnostics_export");
      if (!result.ok) throw new Error(result.message);
      toast(`Diagnostics exported to ${result.data?.path ?? "file"}`);
    }).catch((error) => toast(String(error?.message ?? error), "bad"));
  }
}

let pendingEventRefresh = null;

function queueEventRefresh(delay) {
  window.clearTimeout(pendingEventRefresh);
  state.eventBridge.refreshing = true;
  render();
  pendingEventRefresh = window.setTimeout(async () => {
    pendingEventRefresh = null;
    const synced = await refresh();
    state.eventBridge.refreshing = false;
    if (synced) state.eventBridge.stale = false;
    render();
  }, delay);
}

function markEventBridgeConnected(payload) {
  state.eventBridge.connected = true;
  state.eventBridge.lastConnectedAt = Date.now();
  state.eventBridge.lastError = null;
  state.eventBridge.subscription = {
    replayed: payload?.replayed ?? 0,
    currentSequence: payload?.current_sequence ?? null,
    cursorSequence: payload?.cursor?.after_sequence ?? null,
  };
}

function markEventBridgeEvent(payload) {
  state.eventBridge.connected = true;
  state.eventBridge.lastError = null;
  const event = {
    sequence: payload?.sequence ?? null,
    type: payload?.type ?? "event",
    payload: payload?.payload ?? {},
    occurredAt: payload?.occurred_at_unix_millis ?? null,
    receivedAt: Date.now(),
  };
  state.eventBridge.lastEvent = event;
  state.eventBridge.events = [
    event,
    ...state.eventBridge.events.filter((item) => item.sequence !== event.sequence),
  ].slice(0, 100);
}

function markEventBridgeGap(payload) {
  state.eventBridge.connected = true;
  state.eventBridge.stale = true;
  state.eventBridge.lastGap = {
    expected_after: payload?.expected_after ?? null,
    actual_next: payload?.actual_next ?? null,
  };
}

function markEventBridgeDisconnected(payload) {
  state.eventBridge.connected = false;
  state.eventBridge.refreshing = false;
  state.eventBridge.lastDisconnectedAt = Date.now();
  state.eventBridge.lastError = payload ? String(payload) : "Agent event stream disconnected";
}

async function installEventListeners() {
  await listen("devrelay-tray-refresh", () => {
    refresh();
  });
  await listen("devrelay-agent-event", (event) => {
    markEventBridgeEvent(event?.payload);
    queueEventRefresh(400);
  });
  await listen("devrelay-agent-connected", (event) => {
    markEventBridgeConnected(event?.payload);
    queueEventRefresh(250);
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

installEventListeners().finally(refresh);
