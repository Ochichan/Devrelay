import assert from "node:assert/strict";
import test from "node:test";
import { statusFromAgentState, statusText, statusTooltip } from "../dist/status.js";

function lease(overrides = {}) {
  return {
    lease_id: "lease-1",
    project_id: "project-a",
    state: "active",
    holder_device_id: "device-a",
    handoff_id: null,
    ...overrides,
  };
}

function handoff(state) {
  return {
    record: {
      handoff_id: "handoff-1",
      project_id: "project-a",
      state,
      source_device_id: "device-a",
      target_device_id: "device-b",
    },
  };
}

test("shows active writer state", () => {
  const status = statusFromAgentState({
    deviceId: "device-a",
    leases: [lease()],
    handoffs: [],
  });

  assert.equal(status.kind, "active");
  assert.equal(statusText(status), "$(check) DevRelay Active");
  assert.equal(statusTooltip(status), "DevRelay: Active writer for project-a");
});

test("shows inactive workspace warning when another device holds the lease", () => {
  const status = statusFromAgentState({
    deviceId: "device-b",
    leases: [lease()],
    handoffs: [],
  });

  assert.equal(status.kind, "inactive");
  assert.match(status.detail, /edits may fork/);
  assert.equal(statusText(status), "$(warning) DevRelay Inactive");
});

test("shows handoff in progress before lease state", () => {
  const status = statusFromAgentState({
    deviceId: "device-a",
    leases: [lease()],
    handoffs: [handoff("target-prepare")],
  });

  assert.equal(status.kind, "handoff");
  assert.equal(statusText(status), "$(sync~spin) DevRelay Handoff");
});

test("shows protection delayed when no lease is visible", () => {
  const status = statusFromAgentState({
    deviceId: "device-a",
    leases: [],
    handoffs: [],
  });

  assert.equal(status.kind, "protection-delayed");
  assert.equal(statusText(status), "$(history) DevRelay Delayed");
});
