export type ConnectionStatusKind =
  | "connecting"
  | "connected"
  | "active"
  | "inactive"
  | "handoff"
  | "protection-delayed"
  | "unavailable";

export interface ConnectionStatus {
  kind: ConnectionStatusKind;
  detail: string;
}

export interface LeaseSummary {
  lease_id: string;
  project_id: string;
  state: string;
  holder_device_id?: string | null;
  handoff_id?: string | null;
}

export interface HandoffSummary {
  record: {
    handoff_id: string;
    project_id: string;
    state: string;
    source_device_id: string;
    target_device_id: string;
  };
}

export interface AgentEditorState {
  deviceId?: string;
  leases: LeaseSummary[];
  handoffs: HandoffSummary[];
}

export function statusText(status: ConnectionStatus): string {
  switch (status.kind) {
    case "active":
      return "$(check) DevRelay Active";
    case "connected":
      return "$(check) DevRelay";
    case "connecting":
      return "$(sync~spin) DevRelay";
    case "handoff":
      return "$(sync~spin) DevRelay Handoff";
    case "inactive":
      return "$(warning) DevRelay Inactive";
    case "protection-delayed":
      return "$(history) DevRelay Delayed";
    case "unavailable":
      return "$(warning) DevRelay";
  }
}

export function statusTooltip(status: ConnectionStatus): string {
  return `DevRelay: ${status.detail}`;
}

export function statusFromAgentState(state: AgentEditorState): ConnectionStatus {
  const handoff = state.handoffs
    .map((entry) => entry.record)
    .find((record) => !["committed", "aborted"].includes(record.state));
  if (handoff) {
    return {
      kind: "handoff",
      detail: `Handoff in progress (${handoff.state}) for ${handoff.project_id}`,
    };
  }

  if (!state.deviceId) {
    return {
      kind: "protection-delayed",
      detail: "Agent connected, but local device identity is not available yet",
    };
  }

  const activeLease = state.leases.find(
    (lease) => lease.state === "active" && lease.holder_device_id === state.deviceId
  );
  if (activeLease) {
    return {
      kind: "active",
      detail: `Active writer for ${activeLease.project_id}`,
    };
  }

  const inactiveLease = state.leases.find(
    (lease) =>
      lease.state === "inactive" ||
      lease.state === "forked" ||
      lease.state === "archived" ||
      (lease.holder_device_id !== undefined &&
        lease.holder_device_id !== null &&
        lease.holder_device_id !== state.deviceId)
  );
  if (inactiveLease) {
    return {
      kind: "inactive",
      detail: `Inactive writer for ${inactiveLease.project_id}; edits may fork from the active device`,
    };
  }

  if (state.leases.length === 0) {
    return {
      kind: "protection-delayed",
      detail: "No writer lease is visible yet; protection state is still catching up",
    };
  }

  return {
    kind: "connected",
    detail: "Agent connected",
  };
}
