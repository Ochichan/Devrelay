import * as vscode from "vscode";
import { AgentClient } from "./agentClient";
import {
  assertContextWithinLimit,
  captureWorkspaceContext,
  contextSummary,
  editorContextUpdateParams,
} from "./contextCapture";
import {
  ConnectionStatus,
  HandoffSummary,
  LeaseSummary,
  statusFromAgentState,
  statusText,
  statusTooltip,
} from "./status";
import {
  captureUnsavedBuffers,
  clearUnsavedBufferCapsule,
  loadUnsavedBufferCapsule,
  restoreUnsavedBufferCapsule,
  storeUnsavedBufferCapsule,
  unsavedBufferSummary,
} from "./unsavedBuffers";

interface AgentHealthResult {
  status: string;
  version?: string;
}

interface SettingsGetResult {
  device_id: string;
}

interface LeasesListResult {
  leases: LeaseSummary[];
}

interface HandoffsListResult {
  handoffs: HandoffSummary[];
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
      try {
        const [settings, leases, handoffs] = await Promise.all([
          client.call<SettingsGetResult>("settings.get"),
          client.call<LeasesListResult>("leases.list"),
          client.call<HandoffsListResult>("handoffs.list", { include_journal: false }),
        ]);
        setStatus(
          statusFromAgentState({
            deviceId: settings.device_id,
            leases: leases.leases,
            handoffs: handoffs.handoffs,
          })
        );
      } catch (statusError) {
        const message = statusError instanceof Error ? statusError.message : String(statusError);
        const detail = health.version
          ? `Agent ${health.status} (${health.version}); editor state delayed: ${message}`
          : `Agent ${health.status}; editor state delayed: ${message}`;
        setStatus({ kind: "protection-delayed", detail });
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatus({ kind: "unavailable", detail: message });
      output.appendLine(`Agent unavailable: ${message}`);
    }
  };

  const unsavedCaptureEnabled = () =>
    vscode.workspace.getConfiguration("devrelay").get<boolean>("captureUnsavedBuffers", false);
  const includeUntitledUnsavedBuffers = () =>
    vscode.workspace
      .getConfiguration("devrelay")
      .get<boolean>("includeUntitledUnsavedBuffers", false);
  const captureUnsavedBuffersToSecret = async () => {
    const capsule = captureUnsavedBuffers(vscode.workspace.textDocuments, {
      includeUntitled: includeUntitledUnsavedBuffers(),
    });
    await storeUnsavedBufferCapsule(context.secrets, capsule);
    output.appendLine(`Unsaved buffers captured locally: ${unsavedBufferSummary(capsule)}`);
    return capsule;
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
      if (unsavedCaptureEnabled()) {
        await captureUnsavedBuffersToSecret();
      }
      void vscode.window.showInformationMessage("DevRelay captured editor context.");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      output.appendLine(`Editor context capture failed: ${message}`);
      void vscode.window.showErrorMessage(`DevRelay context capture failed: ${message}`);
    }
  };

  const captureUnsavedBuffersCommand = async () => {
    if (!unsavedCaptureEnabled()) {
      void vscode.window.showWarningMessage("DevRelay unsaved buffer capture is disabled.");
      return;
    }
    try {
      const capsule = await captureUnsavedBuffersToSecret();
      void vscode.window.showInformationMessage(
        `DevRelay captured ${capsule.buffers.length} unsaved buffers locally.`
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      output.appendLine(`Unsaved buffer capture failed: ${message}`);
      void vscode.window.showErrorMessage(`DevRelay unsaved buffer capture failed: ${message}`);
    }
  };

  const restoreUnsavedBuffersCommand = async () => {
    try {
      const capsule = await loadUnsavedBufferCapsule(context.secrets);
      if (!capsule || capsule.buffers.length === 0) {
        void vscode.window.showInformationMessage("No DevRelay unsaved buffers to restore.");
        return;
      }
      const restored = await restoreUnsavedBufferCapsule(capsule, vscode.workspace, vscode.window);
      await clearUnsavedBufferCapsule(context.secrets);
      output.appendLine(`Unsaved buffers restored as dirty untitled documents: ${restored}`);
      void vscode.window.showInformationMessage(`DevRelay restored ${restored} unsaved buffers.`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      output.appendLine(`Unsaved buffer restore failed: ${message}`);
      void vscode.window.showErrorMessage(`DevRelay unsaved buffer restore failed: ${message}`);
    }
  };

  const openDashboard = () => {
    output.appendLine(`Dashboard opened: ${new Date().toISOString()}`);
    output.appendLine(`Current state: ${lastStatus.detail}`);
    output.show(true);
  };

  context.subscriptions.push(
    output,
    statusBar,
    vscode.commands.registerCommand("devrelay.refreshConnection", refresh),
    vscode.commands.registerCommand("devrelay.captureContext", captureContext),
    vscode.commands.registerCommand("devrelay.captureUnsavedBuffers", captureUnsavedBuffersCommand),
    vscode.commands.registerCommand("devrelay.restoreUnsavedBuffers", restoreUnsavedBuffersCommand),
    vscode.commands.registerCommand("devrelay.openDashboard", openDashboard),
    vscode.commands.registerCommand("devrelay.explainState", () => {
      void vscode.window.showInformationMessage(lastStatus.detail);
    })
  );

  void refresh();
}

export function deactivate(): void {
  // VS Code disposes registered subscriptions from the extension context.
}
