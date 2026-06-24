export type ConnectionStatusKind = "connecting" | "connected" | "unavailable";

export interface ConnectionStatus {
  kind: ConnectionStatusKind;
  detail: string;
}

export function statusText(status: ConnectionStatus): string {
  switch (status.kind) {
    case "connected":
      return "$(check) DevRelay";
    case "connecting":
      return "$(sync~spin) DevRelay";
    case "unavailable":
      return "$(warning) DevRelay";
  }
}

export function statusTooltip(status: ConnectionStatus): string {
  return `DevRelay: ${status.detail}`;
}
