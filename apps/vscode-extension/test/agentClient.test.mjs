import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import * as net from "node:net";
import test from "node:test";
import {
  AgentClient,
  AgentRpcError,
  defaultAgentSocketPath,
  defaultDevRelayHome,
  encodeMessage,
} from "../dist/agentClient.js";

test("default paths follow DevRelay home conventions", () => {
  assert.equal(
    defaultDevRelayHome({}, "darwin", "/Users/dev"),
    "/Users/dev/Library/Application Support/devrelay"
  );
  assert.equal(defaultDevRelayHome({ DEVRELAY_HOME: "/tmp/devrelay" }, "darwin", "/Users/dev"), "/tmp/devrelay");
  assert.equal(defaultAgentSocketPath({ DEVRELAY_HOME: "/tmp/devrelay" }, "darwin", "/Users/dev"), "/tmp/devrelay/agent.sock");
});

test("agent client speaks length-prefixed JSON-RPC", async () => {
  const dir = await mkdtemp(join(tmpdir(), "devrelay-vscode-agent-"));
  const socketPath = join(dir, "agent.sock");
  const server = net.createServer((socket) => {
    const chunks = [];
    socket.on("data", (chunk) => {
      chunks.push(chunk);
      const request = Buffer.concat(chunks);
      if (request.byteLength < 4) return;
      const length = request.readUInt32BE(0);
      if (request.byteLength < length + 4) return;
      const payload = JSON.parse(request.subarray(4, length + 4).toString("utf8"));
      assert.equal(payload.method, "agent.health");
      socket.write(
        encodeMessage(
          Buffer.from(
            JSON.stringify({
              jsonrpc: "2.0",
              id: payload.id,
              result: { status: "ok", version: "0.1.0" },
            }),
            "utf8"
          )
        )
      );
    });
  });

  try {
    await new Promise((resolve) => server.listen(socketPath, resolve));
    const client = new AgentClient({ socketPath, timeoutMs: 1000 });
    const result = await client.call("agent.health");
    assert.deepEqual(result, { status: "ok", version: "0.1.0" });
  } finally {
    await new Promise((resolve) => server.close(resolve));
    await rm(dir, { recursive: true, force: true });
  }
});

test("agent client rejects JSON-RPC error responses", async () => {
  const dir = await mkdtemp(join(tmpdir(), "devrelay-vscode-agent-error-"));
  const socketPath = join(dir, "agent.sock");
  const server = net.createServer((socket) => {
    const chunks = [];
    socket.on("data", (chunk) => {
      chunks.push(chunk);
      const request = Buffer.concat(chunks);
      if (request.byteLength < 4) return;
      const length = request.readUInt32BE(0);
      if (request.byteLength < length + 4) return;
      const payload = JSON.parse(request.subarray(4, length + 4).toString("utf8"));
      socket.write(
        encodeMessage(
          Buffer.from(
            JSON.stringify({
              jsonrpc: "2.0",
              id: payload.id,
              error: { code: -32000, message: "agent unavailable" },
            }),
            "utf8"
          )
        )
      );
    });
  });

  try {
    await new Promise((resolve) => server.listen(socketPath, resolve));
    const client = new AgentClient({ socketPath, timeoutMs: 1000 });
    await assert.rejects(
      client.call("agent.health"),
      (error) =>
        error instanceof AgentRpcError &&
        error.code === -32000 &&
        error.message === "agent unavailable"
    );
  } finally {
    await new Promise((resolve) => server.close(resolve));
    await rm(dir, { recursive: true, force: true });
  }
});
