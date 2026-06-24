import * as vscode from "vscode";
import { AgentClient } from "./agentClient";
import {
  assertContextWithinLimit,
  captureWorkspaceContext,
  contextSummary,
  editorContextUpdateParams,
} from "./contextCapture";
import { ConnectionStatus, statusText, statusTooltip } from "./status";

interface AgentHealthResult {
  status: string;
  version?: string;
}

interface EditorContextUpdateResult {
  accepted: boolean;
  audit_id: number;
  capsule_bytes: number;
  recorded_at_unix_seconds: number;
}

export function activate(context: vscode.ExtensionContext): void {
  const output = vscode.window.createOutputChannel("DevRelay");
  const statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 90);
  const client = new AgentClient();
  let lastStatus: ConnectionStatus = {
    kind: "connecting",
    detail: "Checking local agent connection",
  };

  const setStatus = (status: ConnectionStatus) => {
    lastStatus = status;
    statusBar.text = statusText(status);
    statusBar.tooltip = statusTooltip(status);
    statusBar.command = "devrelay.explainState";
    statusBar.show();
  };

  const refresh = async () => {
    setStatus({ kind: "connecting", detail: "Checking local agent connection" });
    try {
      const health = await client.call<AgentHealthResult>("agent.health");
      const detail = health.version
        ? `Agent ${health.status} (${health.version})`
        : `Agent ${health.status}`;
      setStatus({ kind: "connected", detail });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatus({ kind: "unavailable", detail: message });
      output.appendLine(`Agent unavailable: ${message}`);
    }
  };

  const captureContext = async () => {
    try {
      const capsule = captureWorkspaceContext(vscode);
      const capsuleBytes = assertContextWithinLimit(capsule);
      const result = await client.call<EditorContextUpdateResult>(
        "editor.context.update",
        editorContextUpdateParams(capsule)
      );
      output.appendLine(
        `Editor context captured: ${contextSummary(capsule)}; ${capsuleBytes} bytes; audit ${result.audit_id}`
      );
      void vscode.window.showInformationMessage("DevRelay captured editor context.");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      output.appendLine(`Editor context capture failed: ${message}`);
      void vscode.window.showErrorMessage(`DevRelay context capture failed: ${message}`);
    }
  };

  context.subscriptions.push(
    output,
    statusBar,
    vscode.commands.registerCommand("devrelay.refreshConnection", refresh),
    vscode.commands.registerCommand("devrelay.captureContext", captureContext),
    vscode.commands.registerCommand("devrelay.explainState", () => {
      void vscode.window.showInformationMessage(lastStatus.detail);
    })
  );

  void refresh();
}

export function deactivate(): void {
  // VS Code disposes registered subscriptions from the extension context.
}
