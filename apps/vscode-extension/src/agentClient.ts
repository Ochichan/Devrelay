import * as net from "node:net";
import * as os from "node:os";
import * as path from "node:path";

const DEFAULT_TIMEOUT_MS = 30000;
const MAX_MESSAGE_BYTES = 1024 * 1024;

export interface AgentClientOptions {
  socketPath?: string;
  timeoutMs?: number;
}

export interface RpcEnvelope<T = unknown> {
  jsonrpc: "2.0";
  id: string;
  result?: T;
  error?: {
    code: number;
    message: string;
    data?: unknown;
  };
}

export class AgentRpcError extends Error {
  constructor(
    message: string,
    readonly code?: number,
    readonly data?: unknown
  ) {
    super(message);
    this.name = "AgentRpcError";
  }
}

export function defaultDevRelayHome(
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  homeDir: string = os.homedir()
): string {
  if (env.DEVRELAY_HOME) {
    return env.DEVRELAY_HOME;
  }
  if (platform === "darwin") {
    return path.join(homeDir, "Library", "Application Support", "devrelay");
  }
  if (platform === "win32") {
    return path.join(env.LOCALAPPDATA ?? path.join(homeDir, "AppData", "Local"), "devrelay");
  }
  return path.join(env.XDG_DATA_HOME ?? path.join(homeDir, ".local", "share"), "devrelay");
}

export function defaultAgentSocketPath(
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  homeDir: string = os.homedir()
): string {
  return path.join(defaultDevRelayHome(env, platform, homeDir), "agent.sock");
}

export class AgentClient {
  readonly socketPath: string;
  readonly timeoutMs: number;
  private nextId = 1;

  constructor(options: AgentClientOptions = {}) {
    this.socketPath = options.socketPath ?? defaultAgentSocketPath();
    this.timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  }

  call<T>(method: string, params: unknown = {}): Promise<T> {
    const id = `vscode-${this.nextId++}`;
    const payload = Buffer.from(
      JSON.stringify({
        jsonrpc: "2.0",
        id,
        method,
        params,
      }),
      "utf8"
    );
    return this.exchange<T>(id, payload);
  }

  private exchange<T>(id: string, payload: Buffer): Promise<T> {
    if (payload.byteLength > MAX_MESSAGE_BYTES) {
      return Promise.reject(new AgentRpcError("DevRelay request exceeds IPC size limit"));
    }

    return new Promise<T>((resolve, reject) => {
      const socket = net.createConnection(this.socketPath);
      const timeout = setTimeout(() => {
        socket.destroy();
        reject(new AgentRpcError("Timed out waiting for DevRelay agent"));
      }, this.timeoutMs);
      const chunks: Buffer[] = [];
      let expectedLength: number | null = null;
      let settled = false;

      const settle = (fn: () => void) => {
        if (settled) return;
        settled = true;
        clearTimeout(timeout);
        socket.destroy();
        fn();
      };

      socket.once("connect", () => {
        socket.write(encodeMessage(payload));
      });
      socket.on("data", (chunk) => {
        chunks.push(chunk);
        const buffered = Buffer.concat(chunks);
        if (expectedLength === null && buffered.byteLength >= 4) {
          expectedLength = buffered.readUInt32BE(0);
          if (expectedLength > MAX_MESSAGE_BYTES) {
            settle(() => reject(new AgentRpcError("DevRelay response exceeds IPC size limit")));
            return;
          }
        }
        if (expectedLength !== null && buffered.byteLength >= expectedLength + 4) {
          const responseBytes = buffered.subarray(4, expectedLength + 4);
          try {
            const response = decodeResponse<T>(id, responseBytes);
            settle(() => resolve(response));
          } catch (error) {
            settle(() => reject(toAgentRpcError(error)));
          }
        }
      });
      socket.once("error", (error) => {
        settle(() => reject(new AgentRpcError(error.message)));
      });
    });
  }
}

export function encodeMessage(payload: Buffer): Buffer {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(payload.byteLength, 0);
  return Buffer.concat([length, payload]);
}

function decodeResponse<T>(id: string, responseBytes: Buffer): T {
  const envelope = JSON.parse(responseBytes.toString("utf8")) as RpcEnvelope<T>;
  if (envelope.id !== id) {
    throw new AgentRpcError(`DevRelay response id mismatch: expected ${id}, got ${envelope.id}`);
  }
  if (envelope.error) {
    throw new AgentRpcError(envelope.error.message, envelope.error.code, envelope.error.data);
  }
  return envelope.result as T;
}

function toAgentRpcError(error: unknown): AgentRpcError {
  if (error instanceof AgentRpcError) {
    return error;
  }
  const message = error instanceof Error ? error.message : String(error);
  return new AgentRpcError(message);
}
