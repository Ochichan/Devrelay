import { ConnectionStatusKind } from "./status";

export type EditorEventKind =
  | "text-document-changed"
  | "text-document-saved"
  | "active-editor-changed";

export interface EditorEventRecordParams {
  project: string | null;
  workspace_path: string | null;
  event_kind: EditorEventKind;
  document_uri?: string;
  document_path?: string;
  document_version?: number;
  meaningful_edit: boolean;
}

export interface EditorEventRecordResult {
  project?: string | null;
  source_generation: number;
  aborted_handoffs: Array<{
    handoff_id: string;
    project_id: string;
    state: string;
  }>;
}

interface MinimalUri {
  scheme: string;
  fsPath?: string;
  toString(): string;
}

interface MinimalDocument {
  uri: MinimalUri;
  version?: number;
}

interface MinimalWorkspaceFolder {
  uri: MinimalUri;
}

export function editorEventRecordParams(input: {
  eventKind: EditorEventKind;
  document?: MinimalDocument;
  workspaceFolders?: readonly MinimalWorkspaceFolder[];
  contentChangeCount?: number;
}): EditorEventRecordParams {
  return {
    project: null,
    workspace_path: input.workspaceFolders?.[0]?.uri.fsPath ?? null,
    event_kind: input.eventKind,
    document_uri: input.document?.uri.toString(),
    document_path: input.document?.uri.fsPath,
    document_version: input.document?.version,
    meaningful_edit:
      input.eventKind === "text-document-changed" && (input.contentChangeCount ?? 0) > 0,
  };
}

export function shouldNotifyEditorEvent(params: EditorEventRecordParams): boolean {
  if (params.event_kind === "text-document-changed") {
    return params.meaningful_edit;
  }
  return true;
}

export function shouldWarnHandoffInProgress(
  statusKind: ConnectionStatusKind,
  eventKind: EditorEventKind
): boolean {
  return statusKind === "handoff" && eventKind === "text-document-changed";
}

export function editorEventResultSummary(result: EditorEventRecordResult): string {
  const project = result.project ?? "unregistered workspace";
  return `editor event recorded for ${project}; source generation ${result.source_generation}; aborted ${result.aborted_handoffs.length} handoffs`;
}
