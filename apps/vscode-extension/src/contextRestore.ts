import { CapturedResource, CapturedSelection, CapturedWorkspaceContext } from "./contextCapture";

export interface ContextRestoreResult {
  succeeded: boolean;
  partial: boolean;
  opened_workspace_folder?: string;
  opened_files: string[];
  restored_active_file?: string;
  restored_selections: number;
  restored_breakpoints: number;
  restored_unsaved_buffers: number;
  partial_details: string[];
}

export interface ContextRestoreDriver<Editor = unknown> {
  openWorkspaceFolder(path: string): Promise<void>;
  openFile(resource: CapturedResource, viewColumn?: number): Promise<Editor>;
  setSelections(editor: Editor, selections: CapturedSelection[]): void;
  addBreakpoints(breakpoints: RestorableBreakpoint[]): Promise<number>;
  restoreUnsavedBuffers?: () => Promise<number>;
}

export interface RestorableBreakpoint {
  resource: CapturedResource;
  line: number;
  character: number;
  enabled: boolean;
  condition?: string;
  hit_condition?: string;
  log_message?: string;
}

export async function restoreWorkspaceContext<Editor>(
  capsule: CapturedWorkspaceContext,
  driver: ContextRestoreDriver<Editor>
): Promise<ContextRestoreResult> {
  const result: ContextRestoreResult = {
    succeeded: true,
    partial: false,
    opened_files: [],
    restored_selections: 0,
    restored_breakpoints: 0,
    restored_unsaved_buffers: 0,
    partial_details: [],
  };

  const workspaceFolder = capsule.workspace.folders.find((folder) => folder.path)?.path;
  if (workspaceFolder) {
    try {
      await driver.openWorkspaceFolder(workspaceFolder);
      result.opened_workspace_folder = workspaceFolder;
    } catch (error) {
      markPartial(result, `workspace folder restore failed: ${errorMessage(error)}`);
    }
  }

  const opened = new Map<string, Editor>();
  for (const group of capsule.tabs) {
    for (const tab of group.tabs) {
      const resource = firstFileResource(tab.resources);
      if (!resource) {
        continue;
      }
      const key = resourceKey(resource);
      if (opened.has(key)) {
        continue;
      }
      try {
        const editor = await driver.openFile(resource, group.view_column);
        opened.set(key, editor);
        result.opened_files.push(key);
      } catch (error) {
        markPartial(result, `file restore failed (${key}): ${errorMessage(error)}`);
      }
    }
  }

  if (capsule.active_editor?.resource) {
    const key = resourceKey(capsule.active_editor.resource);
    try {
      const editor =
        opened.get(key) ??
        (await driver.openFile(capsule.active_editor.resource, capsule.active_editor.view_column));
      driver.setSelections(editor, capsule.active_editor.selections);
      result.restored_active_file = key;
      result.restored_selections = capsule.active_editor.selections.length;
    } catch (error) {
      markPartial(result, `active editor restore failed (${key}): ${errorMessage(error)}`);
    }
  }

  const breakpoints = capsule.breakpoints.filter(
    (breakpoint): breakpoint is RestorableBreakpoint => breakpoint.resource !== undefined
  );
  if (breakpoints.length > 0) {
    try {
      result.restored_breakpoints = await driver.addBreakpoints(breakpoints);
    } catch (error) {
      markPartial(result, `breakpoint restore failed: ${errorMessage(error)}`);
    }
  }

  if (driver.restoreUnsavedBuffers) {
    try {
      result.restored_unsaved_buffers = await driver.restoreUnsavedBuffers();
    } catch (error) {
      markPartial(result, `unsaved buffer restore failed: ${errorMessage(error)}`);
    }
  }

  return result;
}

function firstFileResource(resources: CapturedResource[]): CapturedResource | undefined {
  return resources.find((resource) => resource.path || resource.uri);
}

export function resourceKey(resource: CapturedResource): string {
  return resource.path ?? resource.uri ?? `${resource.scheme}:unknown`;
}

function markPartial(result: ContextRestoreResult, detail: string): void {
  result.succeeded = false;
  result.partial = true;
  result.partial_details.push(detail);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
