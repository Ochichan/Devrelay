import * as vscode from "vscode";
import { AgentClient } from "./agentClient";
import {
  CapturedResource,
  CapturedSelection,
  CapturedWorkspaceContext,
  assertContextWithinLimit,
  captureWorkspaceContext,
  contextSummary,
  editorContextUpdateParams,
} from "./contextCapture";
import {
  ContextRestoreDriver,
  ContextRestoreResult,
  RestorableBreakpoint,
  restoreWorkspaceContext,
} from "./contextRestore";
import {
  EditorEventRecordParams,
  EditorEventRecordResult,
  editorEventRecordParams,
  editorEventResultSummary,
  shouldNotifyEditorEvent,
  shouldWarnHandoffInProgress,
} from "./editGuard";
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

interface EditorContextSnapshot {
  project?: string | null;
  audit_id: number;
  capsule: CapturedWorkspaceContext;
}

interface EditorContextLatestResult {
  context?: EditorContextSnapshot | null;
}

interface EditorContextUpdateResult {
  accepted: boolean;
  audit_id: number;
  capsule_bytes: number;
  recorded_at_unix_seconds: number;
}

interface EditorRestoreAckResult {
  accepted: boolean;
  audit_id: number;
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

  const restoreUnsavedBuffersFromSecret = async () => {
    const capsule = await loadUnsavedBufferCapsule(context.secrets);
    if (!capsule || capsule.buffers.length === 0) {
      return 0;
    }
    const restored = await restoreUnsavedBufferCapsule(capsule, vscode.workspace, vscode.window);
    await clearUnsavedBufferCapsule(context.secrets);
    return restored;
  };

  const restoreContext = async () => {
    let latest: EditorContextLatestResult | undefined;
    let restoreResult: ContextRestoreResult | undefined;
    try {
      latest = await client.call<EditorContextLatestResult>("editor.context.latest", {
        project: null,
      });
      if (!latest.context) {
        void vscode.window.showInformationMessage("No DevRelay editor context to restore.");
        return;
      }
      restoreResult = await restoreWorkspaceContext(
        latest.context.capsule,
        createContextRestoreDriver(restoreUnsavedBuffersFromSecret)
      );
      await client.call<EditorRestoreAckResult>("editor.restore.ack", {
        project: latest.context.project ?? null,
        restored_context_audit_id: latest.context.audit_id,
        succeeded: restoreResult.succeeded,
        partial: restoreResult.partial,
        detail: restoreResult,
      });
      output.appendLine(`Editor context restore result: ${JSON.stringify(restoreResult)}`);
      void vscode.window.showInformationMessage(
        restoreResult.partial
          ? "DevRelay partially restored editor context."
          : "DevRelay restored editor context."
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      output.appendLine(`Editor context restore failed: ${message}`);
      if (latest?.context) {
        void client.call<EditorRestoreAckResult>("editor.restore.ack", {
          project: latest.context.project ?? null,
          restored_context_audit_id: latest.context.audit_id,
          succeeded: false,
          partial: true,
          detail: restoreResult ?? { error: message },
        });
      }
      void vscode.window.showErrorMessage(`DevRelay context restore failed: ${message}`);
    }
  };

  const recordEditorEvent = async (params: EditorEventRecordParams) => {
    if (!shouldNotifyEditorEvent(params)) {
      return;
    }
    const warnBeforeRecord = shouldWarnHandoffInProgress(lastStatus.kind, params.event_kind);
    try {
      const result = await client.call<EditorEventRecordResult>("editor.event.record", params);
      output.appendLine(editorEventResultSummary(result));
      if (result.aborted_handoffs.length > 0) {
        void vscode.window.showWarningMessage(
          "DevRelay aborted the active handoff because this workspace changed."
        );
        void refresh();
      } else if (warnBeforeRecord) {
        void vscode.window.showWarningMessage(
          "DevRelay recorded an edit while handoff was in progress."
        );
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      output.appendLine(`Editor event record failed: ${message}`);
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
    vscode.commands.registerCommand("devrelay.restoreContext", restoreContext),
    vscode.commands.registerCommand("devrelay.explainState", () => {
      void vscode.window.showInformationMessage(lastStatus.detail);
    }),
    vscode.workspace.onDidChangeTextDocument((event) => {
      void recordEditorEvent(
        editorEventRecordParams({
          eventKind: "text-document-changed",
          document: event.document,
          workspaceFolders: vscode.workspace.workspaceFolders,
          contentChangeCount: event.contentChanges.length,
        })
      );
    }),
    vscode.workspace.onDidSaveTextDocument((document) => {
      void recordEditorEvent(
        editorEventRecordParams({
          eventKind: "text-document-saved",
          document,
          workspaceFolders: vscode.workspace.workspaceFolders,
        })
      );
    }),
    vscode.window.onDidChangeActiveTextEditor((editor) => {
      void recordEditorEvent(
        editorEventRecordParams({
          eventKind: "active-editor-changed",
          document: editor?.document,
          workspaceFolders: vscode.workspace.workspaceFolders,
        })
      );
    })
  );

  void refresh();
}

function createContextRestoreDriver(
  restoreUnsavedBuffers: () => Promise<number>
): ContextRestoreDriver<vscode.TextEditor> {
  return {
    openWorkspaceFolder: async (path) => {
      const alreadyOpen = vscode.workspace.workspaceFolders?.some(
        (folder) => folder.uri.fsPath === path
      );
      if (!alreadyOpen) {
        const insertAt = vscode.workspace.workspaceFolders?.length ?? 0;
        const added = vscode.workspace.updateWorkspaceFolders(insertAt, 0, {
          uri: vscode.Uri.file(path),
        });
        if (!added) {
          throw new Error(`VS Code rejected workspace folder ${path}`);
        }
      }
    },
    openFile: async (resource, viewColumn) => {
      const document = await vscode.workspace.openTextDocument(uriFromCapturedResource(resource));
      return vscode.window.showTextDocument(document, {
        preview: false,
        viewColumn: viewColumn as vscode.ViewColumn | undefined,
      });
    },
    setSelections: (editor, selections) => {
      const restored = selections.map(selectionFromCaptured);
      editor.selections = restored;
      if (restored[0]) {
        editor.selection = restored[0];
      }
    },
    addBreakpoints: async (breakpoints) => {
      const restored = breakpoints.map(sourceBreakpointFromCaptured);
      vscode.debug.addBreakpoints(restored);
      return restored.length;
    },
    restoreUnsavedBuffers,
  };
}

function uriFromCapturedResource(resource: CapturedResource): vscode.Uri {
  if (resource.path) {
    return vscode.Uri.file(resource.path);
  }
  if (resource.uri) {
    return vscode.Uri.parse(resource.uri);
  }
  return vscode.Uri.parse(`${resource.scheme}:`);
}

function selectionFromCaptured(selection: CapturedSelection): vscode.Selection {
  return new vscode.Selection(
    new vscode.Position(selection.anchor.line, selection.anchor.character),
    new vscode.Position(selection.active.line, selection.active.character)
  );
}

function sourceBreakpointFromCaptured(breakpoint: RestorableBreakpoint): vscode.SourceBreakpoint {
  return new vscode.SourceBreakpoint(
    new vscode.Location(
      uriFromCapturedResource(breakpoint.resource),
      new vscode.Position(breakpoint.line, breakpoint.character)
    ),
    breakpoint.enabled,
    breakpoint.condition,
    breakpoint.hit_condition,
    breakpoint.log_message
  );
}

export function deactivate(): void {
  // VS Code disposes registered subscriptions from the extension context.
}
