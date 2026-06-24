export const UNSAVED_BUFFER_SCHEMA_VERSION = 1;
export const UNSAVED_BUFFER_SOURCE = "vscode";
export const UNSAVED_BUFFER_SECRET_KEY = "devrelay.unsavedBuffers.latest";

export interface UnsavedBufferCaptureOptions {
  includeUntitled?: boolean;
  maxBuffers?: number;
  maxBufferBytes?: number;
  maxTotalBytes?: number;
  now?: () => number;
}

export interface UnsavedBufferCapsule {
  schema_version: number;
  source: typeof UNSAVED_BUFFER_SOURCE;
  captured_at_unix_millis: number;
  local_only: true;
  storage: "vscode.SecretStorage";
  buffers: UnsavedBufferRecord[];
  excluded: UnsavedBufferExcludedRecord[];
  limits: Required<Omit<UnsavedBufferCaptureOptions, "now" | "includeUntitled">> & {
    includeUntitled: boolean;
  };
}

export interface UnsavedBufferRecord {
  uri: string;
  path?: string;
  scheme: string;
  language_id: string;
  version: number;
  is_untitled: boolean;
  text: string;
  text_bytes: number;
}

export interface UnsavedBufferExcludedRecord {
  uri: string;
  scheme: string;
  reason: "clean" | "untitled-disabled" | "buffer-too-large" | "total-too-large" | "buffer-limit";
}

interface MinimalTextDocument {
  uri: MinimalUri;
  isDirty: boolean;
  isUntitled: boolean;
  languageId: string;
  version: number;
  getText(): string;
}

interface MinimalUri {
  scheme: string;
  fsPath?: string;
  toString(): string;
}

interface MinimalSecretStorage {
  get(key: string): PromiseLike<string | undefined> | Promise<string | undefined>;
  store(key: string, value: string): PromiseLike<void> | Promise<void>;
  delete(key: string): PromiseLike<void> | Promise<void>;
}

interface MinimalWorkspace {
  openTextDocument(options: { language?: string; content?: string }): PromiseLike<unknown> | Promise<unknown>;
}

interface MinimalWindow {
  showTextDocument(document: unknown, options?: { preview?: boolean }): PromiseLike<unknown> | Promise<unknown>;
}

const DEFAULT_UNSAVED_BUFFER_CAPTURE_OPTIONS = {
  includeUntitled: false,
  maxBuffers: 16,
  maxBufferBytes: 128 * 1024,
  maxTotalBytes: 512 * 1024,
};

export function captureUnsavedBuffers(
  documents: readonly MinimalTextDocument[],
  options: UnsavedBufferCaptureOptions = {}
): UnsavedBufferCapsule {
  const normalized = normalizeOptions(options);
  const buffers: UnsavedBufferRecord[] = [];
  const excluded: UnsavedBufferExcludedRecord[] = [];
  let totalBytes = 0;

  for (const document of documents) {
    const uri = document.uri.toString();
    const scheme = document.uri.scheme;
    if (!document.isDirty) {
      excluded.push({ uri, scheme, reason: "clean" });
      continue;
    }
    if (document.isUntitled && !normalized.includeUntitled) {
      excluded.push({ uri, scheme, reason: "untitled-disabled" });
      continue;
    }
    if (buffers.length >= normalized.maxBuffers) {
      excluded.push({ uri, scheme, reason: "buffer-limit" });
      continue;
    }

    const text = document.getText();
    const textBytes = Buffer.byteLength(text, "utf8");
    if (textBytes > normalized.maxBufferBytes) {
      excluded.push({ uri, scheme, reason: "buffer-too-large" });
      continue;
    }
    if (totalBytes + textBytes > normalized.maxTotalBytes) {
      excluded.push({ uri, scheme, reason: "total-too-large" });
      continue;
    }
    totalBytes += textBytes;
    buffers.push({
      uri,
      path: document.uri.fsPath || undefined,
      scheme,
      language_id: document.languageId,
      version: document.version,
      is_untitled: document.isUntitled,
      text,
      text_bytes: textBytes,
    });
  }

  return {
    schema_version: UNSAVED_BUFFER_SCHEMA_VERSION,
    source: UNSAVED_BUFFER_SOURCE,
    captured_at_unix_millis: (options.now ?? Date.now)(),
    local_only: true,
    storage: "vscode.SecretStorage",
    buffers,
    excluded,
    limits: normalized,
  };
}

export async function storeUnsavedBufferCapsule(
  secrets: MinimalSecretStorage,
  capsule: UnsavedBufferCapsule
): Promise<void> {
  await secrets.store(UNSAVED_BUFFER_SECRET_KEY, JSON.stringify(capsule));
}

export async function loadUnsavedBufferCapsule(
  secrets: MinimalSecretStorage
): Promise<UnsavedBufferCapsule | undefined> {
  const raw = await secrets.get(UNSAVED_BUFFER_SECRET_KEY);
  if (!raw) {
    return undefined;
  }
  return JSON.parse(raw) as UnsavedBufferCapsule;
}

export async function clearUnsavedBufferCapsule(secrets: MinimalSecretStorage): Promise<void> {
  await secrets.delete(UNSAVED_BUFFER_SECRET_KEY);
}

export async function restoreUnsavedBufferCapsule(
  capsule: UnsavedBufferCapsule,
  workspace: MinimalWorkspace,
  window: MinimalWindow
): Promise<number> {
  let restored = 0;
  for (const buffer of capsule.buffers) {
    const document = await workspace.openTextDocument({
      language: buffer.language_id,
      content: buffer.text,
    });
    await window.showTextDocument(document, { preview: false });
    restored += 1;
  }
  return restored;
}

export function unsavedBufferSummary(capsule: UnsavedBufferCapsule): string {
  return `${capsule.buffers.length} dirty buffers, ${capsule.excluded.length} excluded`;
}

function normalizeOptions(
  options: UnsavedBufferCaptureOptions
): Required<Omit<UnsavedBufferCaptureOptions, "now" | "includeUntitled">> & {
  includeUntitled: boolean;
} {
  return {
    includeUntitled: options.includeUntitled ?? DEFAULT_UNSAVED_BUFFER_CAPTURE_OPTIONS.includeUntitled,
    maxBuffers: options.maxBuffers ?? DEFAULT_UNSAVED_BUFFER_CAPTURE_OPTIONS.maxBuffers,
    maxBufferBytes: options.maxBufferBytes ?? DEFAULT_UNSAVED_BUFFER_CAPTURE_OPTIONS.maxBufferBytes,
    maxTotalBytes: options.maxTotalBytes ?? DEFAULT_UNSAVED_BUFFER_CAPTURE_OPTIONS.maxTotalBytes,
  };
}
