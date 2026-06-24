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

interface ProjectRegistryEntry {
  project_id: string;
  display_name: string;
  local_path: string;
  manifest_path?: string | null;
}

interface ProjectsListResult {
  projects: ProjectRegistryEntry[];
}

interface DeviceIdentity {
  device_id: string;
  display_name: string;
  platform_key: string;
  architecture: string;
}

interface DevicesListResult {
  devices: DeviceIdentity[];
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

interface CheckpointCreateResult {
  checkpoint: {
    snapshot_id: string;
    project_id: string;
    label?: string | null;
  };
  snapshot_repo: string;
}

interface StoredSnapshotSummary {
  snapshot_id: string;
  project_id: string;
  label?: string | null;
  created_at_unix_seconds: number;
}

interface RecoverListResult {
  snapshots: StoredSnapshotSummary[];
}

interface TaskRunRecord {
  task_run_id: string;
  project_id: string;
  state: string;
  command?: string | null;
  updated_at_unix_seconds: number;
}

interface RunsListResult {
  runs: TaskRunRecord[];
}

interface HandoffMutationResult {
  handoff: {
    handoff_id: string;
    lease_id: string;
    project_id: string;
    target_device_id: string;
    source_generation: string;
    state: string;
  };
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

  const reportCommandError = (label: string, error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    output.appendLine(`${label} failed: ${message}`);
    void vscode.window.showErrorMessage(`DevRelay ${label.toLowerCase()} failed: ${message}`);
  };

  const captureContext = async (): Promise<EditorContextUpdateResult | undefined> => {
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
      return result;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      output.appendLine(`Editor context capture failed: ${message}`);
      void vscode.window.showErrorMessage(`DevRelay context capture failed: ${message}`);
      return undefined;
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

  const createCheckpoint = async () => {
    const repo = currentWorkspacePath();
    if (!repo) {
      void vscode.window.showWarningMessage("Open a workspace folder before creating a checkpoint.");
      return;
    }
    try {
      const result = await client.call<CheckpointCreateResult>("checkpoint.create", {
        repo,
        manifest: null,
        label: "VS Code checkpoint",
        pin: false,
      });
      output.appendLine(
        `Checkpoint created: ${result.checkpoint.snapshot_id} for ${result.checkpoint.project_id}`
      );
      void vscode.window.showInformationMessage("DevRelay checkpoint created.");
      void refresh();
    } catch (error) {
      reportCommandError("Checkpoint", error);
    }
  };

  const openRecoveryTimeline = async () => {
    try {
      const result = await client.call<RecoverListResult>("recover.list", {
        project: null,
      });
      output.appendLine("Recovery timeline:");
      if (result.snapshots.length === 0) {
        output.appendLine("- No recovery snapshots available.");
      }
      for (const snapshot of result.snapshots.slice(0, 25)) {
        output.appendLine(
          `- ${snapshot.snapshot_id} project=${snapshot.project_id} label=${snapshot.label ?? "-"} created=${formatUnixSeconds(
            snapshot.created_at_unix_seconds
          )}`
        );
      }
      output.show(true);
      void vscode.window.showInformationMessage("DevRelay recovery timeline opened.");
    } catch (error) {
      reportCommandError("Open recovery timeline", error);
    }
  };

  const runTask = async () => {
    try {
      const projects = await client.call<ProjectsListResult>("projects.list");
      const project = await selectProjectForWorkspace(projects.projects, currentWorkspacePath());
      if (!project) {
        return;
      }
      const result = await client.call<RunsListResult>("runs.list", {
        project: project.project_id,
        limit: 10,
      });
      output.appendLine(`Recent task runs for ${project.display_name} (${project.project_id}):`);
      if (result.runs.length === 0) {
        output.appendLine("- No task runs recorded.");
      }
      for (const run of result.runs) {
        output.appendLine(
          `- ${run.task_run_id} state=${run.state} command=${run.command ?? "-"} updated=${formatUnixSeconds(
            run.updated_at_unix_seconds
          )}`
        );
      }
      output.show(true);
      void vscode.window.showWarningMessage(
        "DevRelay task execution is not available from the local agent yet; opened recent runs."
      );
    } catch (error) {
      reportCommandError("Run task", error);
    }
  };

  const continueHere = async () => {
    await restoreContext();
    void refresh();
  };

  const continueElsewhere = async () => {
    const workspacePath = currentWorkspacePath();
    if (!workspacePath) {
      void vscode.window.showWarningMessage("Open a workspace folder before continuing elsewhere.");
      return;
    }
    try {
      const [settings, projects, devices] = await Promise.all([
        client.call<SettingsGetResult>("settings.get"),
        client.call<ProjectsListResult>("projects.list"),
        client.call<DevicesListResult>("devices.list"),
      ]);
      const project = await selectProjectForWorkspace(projects.projects, workspacePath);
      if (!project) {
        return;
      }
      const target = await selectTargetDevice(devices.devices, settings.device_id);
      if (!target) {
        return;
      }
      const leases = await client.call<LeasesListResult>("leases.list", {
        project: project.project_id,
      });
      const lease = leases.leases.find(
        (entry) =>
          entry.project_id === project.project_id &&
          entry.state === "active" &&
          entry.holder_device_id === settings.device_id
      );
      if (!lease) {
        void vscode.window.showWarningMessage(
          "DevRelay cannot continue elsewhere because this device does not hold an active writer lease."
        );
        return;
      }
      const captured = await captureContext();
      if (!captured) {
        return;
      }
      const handoff = await client.call<HandoffMutationResult>("handoff.begin", {
        project: project.project_id,
        lease_id: lease.lease_id,
        target_device_id: target.device_id,
        source_generation: `vscode-context-${captured.audit_id}`,
        ttl_seconds: null,
      });
      output.appendLine(
        `Handoff started: ${handoff.handoff.handoff_id} to ${target.display_name} (${target.device_id})`
      );
      void vscode.window.showInformationMessage("DevRelay handoff started.");
      void refresh();
    } catch (error) {
      reportCommandError("Continue elsewhere", error);
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
    vscode.commands.registerCommand("devrelay.continueHere", continueHere),
    vscode.commands.registerCommand("devrelay.continueElsewhere", continueElsewhere),
    vscode.commands.registerCommand("devrelay.captureContext", captureContext),
    vscode.commands.registerCommand("devrelay.createCheckpoint", createCheckpoint),
    vscode.commands.registerCommand("devrelay.runTask", runTask),
    vscode.commands.registerCommand("devrelay.openRecoveryTimeline", openRecoveryTimeline),
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

function currentWorkspacePath(): string | undefined {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

async function selectProjectForWorkspace(
  projects: ProjectRegistryEntry[],
  workspacePath: string | undefined
): Promise<ProjectRegistryEntry | undefined> {
  if (projects.length === 0) {
    void vscode.window.showWarningMessage("No DevRelay projects are registered with the agent.");
    return undefined;
  }
  if (workspacePath) {
    const matched = projects.find((project) => projectMatchesWorkspace(project, workspacePath));
    if (matched) {
      return matched;
    }
  }
  if (projects.length === 1) {
    return projects[0];
  }
  const selected = await vscode.window.showQuickPick(
    projects.map((project) => ({
      label: project.display_name,
      description: project.project_id,
      detail: project.local_path,
      project,
    })),
    { placeHolder: "Select a DevRelay project" }
  );
  return selected?.project;
}

async function selectTargetDevice(
  devices: DeviceIdentity[],
  currentDeviceId: string
): Promise<DeviceIdentity | undefined> {
  const targets = devices.filter((device) => device.device_id !== currentDeviceId);
  if (targets.length === 0) {
    void vscode.window.showWarningMessage("No other DevRelay devices are available.");
    return undefined;
  }
  if (targets.length === 1) {
    return targets[0];
  }
  const selected = await vscode.window.showQuickPick(
    targets.map((device) => ({
      label: device.display_name,
      description: device.device_id,
      detail: `${device.platform_key} ${device.architecture}`,
      device,
    })),
    { placeHolder: "Select a target device" }
  );
  return selected?.device;
}

function projectMatchesWorkspace(project: ProjectRegistryEntry, workspacePath: string): boolean {
  return workspacePath === project.local_path || workspacePath.startsWith(`${project.local_path}/`);
}

function formatUnixSeconds(seconds: number): string {
  return new Date(seconds * 1000).toISOString();
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
