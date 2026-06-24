export const EDITOR_CONTEXT_SCHEMA_VERSION = 1;
export const EDITOR_CONTEXT_SOURCE = "vscode";

export interface ContextCaptureLimits {
  max_workspace_folders: number;
  max_tab_groups: number;
  max_tabs: number;
  max_breakpoints: number;
  max_terminals: number;
  max_selections: number;
  max_string_chars: number;
  max_capsule_bytes: number;
}

export interface ContextCaptureOptions {
  limits?: Partial<ContextCaptureLimits>;
  now?: () => number;
}

export interface CapturedResource {
  scheme: string;
  path?: string;
  uri?: string;
}

export interface CapturedPosition {
  line: number;
  character: number;
}

export interface CapturedSelection {
  anchor: CapturedPosition;
  active: CapturedPosition;
  start: CapturedPosition;
  end: CapturedPosition;
}

export interface CapturedWorkspaceContext {
  schema_version: number;
  source: typeof EDITOR_CONTEXT_SOURCE;
  captured_at_unix_millis: number;
  workspace: {
    name?: string;
    folders: Array<{
      name: string;
      index: number;
      path?: string;
      uri?: string;
    }>;
  };
  tabs: Array<{
    view_column?: number;
    is_active: boolean;
    active_tab_index?: number;
    tabs: Array<{
      label: string;
      input_kind: string;
      is_active: boolean;
      is_dirty: boolean;
      is_pinned: boolean;
      is_preview: boolean;
      resources: CapturedResource[];
    }>;
  }>;
  active_editor?: {
    resource?: CapturedResource;
    view_column?: number;
    cursor: CapturedPosition;
    selections: CapturedSelection[];
  };
  breakpoints: Array<{
    resource?: CapturedResource;
    line: number;
    character: number;
    enabled: boolean;
    condition?: string;
    hit_condition?: string;
    log_message?: string;
  }>;
  terminals: Array<{
    title: string;
    cwd?: CapturedResource;
    is_interacted_with?: boolean;
  }>;
  limits: ContextCaptureLimits & {
    truncated: string[];
  };
}

interface MinimalVsCodeApi {
  workspace: {
    name?: string;
    workspaceFolders?: readonly unknown[];
  };
  window: {
    activeTextEditor?: unknown;
    tabGroups?: {
      all?: readonly unknown[];
    };
    terminals?: readonly unknown[];
  };
  debug: {
    breakpoints?: readonly unknown[];
  };
}

export const DEFAULT_CONTEXT_CAPTURE_LIMITS: ContextCaptureLimits = {
  max_workspace_folders: 8,
  max_tab_groups: 8,
  max_tabs: 64,
  max_breakpoints: 64,
  max_terminals: 16,
  max_selections: 16,
  max_string_chars: 4096,
  max_capsule_bytes: 128 * 1024,
};

export function captureWorkspaceContext(
  vscode: MinimalVsCodeApi,
  options: ContextCaptureOptions = {}
): CapturedWorkspaceContext {
  const limits = normalizeLimits(options.limits);
  const truncated: string[] = [];
  const now = options.now ?? Date.now;
  const workspaceFolders = takeLimited(
    vscode.workspace.workspaceFolders ?? [],
    limits.max_workspace_folders,
    "workspace.folders",
    truncated
  );
  const tabGroups = takeLimited(
    vscode.window.tabGroups?.all ?? [],
    limits.max_tab_groups,
    "tab.groups",
    truncated
  );
  const breakpoints = takeLimited(
    vscode.debug.breakpoints ?? [],
    limits.max_breakpoints,
    "breakpoints",
    truncated
  );
  const terminals = takeLimited(
    vscode.window.terminals ?? [],
    limits.max_terminals,
    "terminals",
    truncated
  );

  return {
    schema_version: EDITOR_CONTEXT_SCHEMA_VERSION,
    source: EDITOR_CONTEXT_SOURCE,
    captured_at_unix_millis: now(),
    workspace: {
      name: limitString(vscode.workspace.name, limits, "workspace.name", truncated),
      folders: workspaceFolders.map((folder) => workspaceFolderFrom(folder, limits, truncated)),
    },
    tabs: tabGroups.map((group) => tabGroupFrom(group, limits, truncated)),
    active_editor: activeEditorFrom(vscode.window.activeTextEditor, limits, truncated),
    breakpoints: breakpoints.flatMap((breakpoint) => breakpointFrom(breakpoint, limits, truncated)),
    terminals: terminals.map((terminal) => terminalFrom(terminal, limits, truncated)),
    limits: {
      ...limits,
      truncated,
    },
  };
}

export function editorContextUpdateParams(capsule: CapturedWorkspaceContext): {
  project: null;
  workspace_path: string | null;
  capsule: CapturedWorkspaceContext;
} {
  return {
    project: null,
    workspace_path: capsule.workspace.folders[0]?.path ?? null,
    capsule,
  };
}

export function assertContextWithinLimit(
  capsule: CapturedWorkspaceContext,
  limits: Partial<ContextCaptureLimits> = {}
): number {
  const maxBytes = normalizeLimits(limits).max_capsule_bytes;
  const bytes = contextByteLength(capsule);
  if (bytes > maxBytes) {
    throw new Error(`Editor context capsule is ${bytes} bytes; limit is ${maxBytes} bytes`);
  }
  return bytes;
}

export function contextByteLength(capsule: CapturedWorkspaceContext): number {
  return Buffer.byteLength(JSON.stringify(capsule), "utf8");
}

export function contextSummary(capsule: CapturedWorkspaceContext): string {
  const folderCount = capsule.workspace.folders.length;
  const tabCount = capsule.tabs.reduce((count, group) => count + group.tabs.length, 0);
  const breakpointCount = capsule.breakpoints.length;
  const terminalCount = capsule.terminals.length;
  return `${folderCount} folders, ${tabCount} tabs, ${breakpointCount} breakpoints, ${terminalCount} terminals`;
}

function normalizeLimits(limits: Partial<ContextCaptureLimits> = {}): ContextCaptureLimits {
  return {
    ...DEFAULT_CONTEXT_CAPTURE_LIMITS,
    ...limits,
  };
}

function workspaceFolderFrom(
  folder: unknown,
  limits: ContextCaptureLimits,
  truncated: string[]
): CapturedWorkspaceContext["workspace"]["folders"][number] {
  const record = asRecord(folder);
  const resource = resourceFromUri(record?.uri, limits, "workspace.folder", truncated);
  return {
    name: limitString(readString(record, "name") ?? "", limits, "workspace.folder.name", truncated) ?? "",
    index: readNumber(record, "index") ?? 0,
    path: resource?.path,
    uri: resource?.uri,
  };
}

function tabGroupFrom(
  group: unknown,
  limits: ContextCaptureLimits,
  truncated: string[]
): CapturedWorkspaceContext["tabs"][number] {
  const record = asRecord(group);
  const tabs = takeLimited(readArray(record, "tabs"), limits.max_tabs, "tabs", truncated);
  return {
    view_column: readNumber(record, "viewColumn"),
    is_active: readBoolean(record, "isActive") ?? false,
    active_tab_index: activeTabIndex(record, tabs),
    tabs: tabs.map((tab) => tabFrom(tab, limits, truncated)),
  };
}

function activeTabIndex(group: Record<string, unknown> | undefined, tabs: readonly unknown[]): number | undefined {
  const activeTab = group?.activeTab;
  const index = tabs.findIndex((tab) => tab === activeTab);
  return index >= 0 ? index : undefined;
}

function tabFrom(
  tab: unknown,
  limits: ContextCaptureLimits,
  truncated: string[]
): CapturedWorkspaceContext["tabs"][number]["tabs"][number] {
  const record = asRecord(tab);
  const input = record?.input;
  return {
    label: limitString(readString(record, "label") ?? "", limits, "tab.label", truncated) ?? "",
    input_kind: inputKind(input),
    is_active: readBoolean(record, "isActive") ?? false,
    is_dirty: readBoolean(record, "isDirty") ?? false,
    is_pinned: readBoolean(record, "isPinned") ?? false,
    is_preview: readBoolean(record, "isPreview") ?? false,
    resources: resourcesFromInput(input, limits, truncated),
  };
}

function activeEditorFrom(
  editor: unknown,
  limits: ContextCaptureLimits,
  truncated: string[]
): CapturedWorkspaceContext["active_editor"] {
  const record = asRecord(editor);
  if (!record) {
    return undefined;
  }
  const document = asRecord(record.document);
  const selection = asRecord(record.selection);
  return {
    resource: resourceFromUri(document?.uri, limits, "active_editor.resource", truncated),
    view_column: readNumber(record, "viewColumn"),
    cursor: positionFrom(selection?.active),
    selections: takeLimited(
      readArray(record, "selections"),
      limits.max_selections,
      "active_editor.selections",
      truncated
    ).map(selectionFrom),
  };
}

function selectionFrom(selection: unknown): CapturedSelection {
  const record = asRecord(selection);
  return {
    anchor: positionFrom(record?.anchor),
    active: positionFrom(record?.active),
    start: positionFrom(record?.start),
    end: positionFrom(record?.end),
  };
}

function breakpointFrom(
  breakpoint: unknown,
  limits: ContextCaptureLimits,
  truncated: string[]
): CapturedWorkspaceContext["breakpoints"] {
  const record = asRecord(breakpoint);
  const location = asRecord(record?.location);
  if (!location) {
    return [];
  }
  const range = asRecord(location.range);
  const start = positionFrom(range?.start);
  return [
    {
      resource: resourceFromUri(location.uri, limits, "breakpoint.resource", truncated),
      line: start.line,
      character: start.character,
      enabled: readBoolean(record, "enabled") ?? true,
      condition: limitString(readString(record, "condition"), limits, "breakpoint.condition", truncated),
      hit_condition: limitString(readString(record, "hitCondition"), limits, "breakpoint.hitCondition", truncated),
      log_message: limitString(readString(record, "logMessage"), limits, "breakpoint.logMessage", truncated),
    },
  ];
}

function terminalFrom(
  terminal: unknown,
  limits: ContextCaptureLimits,
  truncated: string[]
): CapturedWorkspaceContext["terminals"][number] {
  const record = asRecord(terminal);
  const state = asRecord(record?.state);
  return {
    title: limitString(readString(record, "name") ?? "", limits, "terminal.name", truncated) ?? "",
    cwd: terminalCwdFrom(record, limits, truncated),
    is_interacted_with: readBoolean(state, "isInteractedWith"),
  };
}

function terminalCwdFrom(
  terminal: Record<string, unknown> | undefined,
  limits: ContextCaptureLimits,
  truncated: string[]
): CapturedResource | undefined {
  const creationOptions = asRecord(terminal?.creationOptions);
  const cwd = creationOptions?.cwd;
  if (typeof cwd === "string") {
    return {
      scheme: "file",
      path: limitString(cwd, limits, "terminal.cwd", truncated),
    };
  }
  return resourceFromUri(cwd, limits, "terminal.cwd", truncated);
}

function resourcesFromInput(
  input: unknown,
  limits: ContextCaptureLimits,
  truncated: string[]
): CapturedResource[] {
  const record = asRecord(input);
  if (!record) {
    return [];
  }
  const resources: CapturedResource[] = [];
  for (const key of ["uri", "original", "modified"]) {
    const resource = resourceFromUri(record[key], limits, `tab.${key}`, truncated);
    if (resource) {
      resources.push(resource);
    }
  }
  return resources;
}

function inputKind(input: unknown): string {
  const record = asRecord(input);
  if (!record) {
    return "unknown";
  }
  if ("original" in record && "modified" in record) {
    return readString(record, "notebookType") ? "notebook-diff" : "diff";
  }
  if ("notebookType" in record) {
    return "notebook";
  }
  if ("viewType" in record) {
    return "custom";
  }
  if ("uri" in record) {
    return "text";
  }
  return "terminal";
}

function resourceFromUri(
  uri: unknown,
  limits: ContextCaptureLimits,
  label: string,
  truncated: string[]
): CapturedResource | undefined {
  const record = asRecord(uri);
  if (!record) {
    return undefined;
  }
  const scheme = readString(record, "scheme") ?? "unknown";
  const fsPath = readString(record, "fsPath");
  const external = typeof record.toString === "function" ? record.toString() : undefined;
  const resource: CapturedResource = {
    scheme: limitString(scheme, limits, `${label}.scheme`, truncated) ?? "unknown",
  };
  if (fsPath) {
    resource.path = limitString(fsPath, limits, `${label}.path`, truncated);
  } else if (typeof external === "string") {
    resource.uri = limitString(external, limits, `${label}.uri`, truncated);
  }
  return resource;
}

function positionFrom(position: unknown): CapturedPosition {
  const record = asRecord(position);
  return {
    line: readNumber(record, "line") ?? 0,
    character: readNumber(record, "character") ?? 0,
  };
}

function takeLimited<T>(
  values: readonly T[],
  limit: number,
  label: string,
  truncated: string[]
): readonly T[] {
  if (values.length <= limit) {
    return values;
  }
  truncated.push(`${label}:${values.length}->${limit}`);
  return values.slice(0, limit);
}

function limitString(
  value: string | undefined,
  limits: ContextCaptureLimits,
  label: string,
  truncated: string[]
): string | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (value.length <= limits.max_string_chars) {
    return value;
  }
  truncated.push(`${label}:${value.length}->${limits.max_string_chars}`);
  return value.slice(0, limits.max_string_chars);
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  if (value && typeof value === "object") {
    return value as Record<string, unknown>;
  }
  return undefined;
}

function readString(record: Record<string, unknown> | undefined, key: string): string | undefined {
  const value = record?.[key];
  return typeof value === "string" ? value : undefined;
}

function readNumber(record: Record<string, unknown> | undefined, key: string): number | undefined {
  const value = record?.[key];
  return typeof value === "number" ? value : undefined;
}

function readBoolean(record: Record<string, unknown> | undefined, key: string): boolean | undefined {
  const value = record?.[key];
  return typeof value === "boolean" ? value : undefined;
}

function readArray(record: Record<string, unknown> | undefined, key: string): readonly unknown[] {
  const value = record?.[key];
  return Array.isArray(value) ? value : [];
}
