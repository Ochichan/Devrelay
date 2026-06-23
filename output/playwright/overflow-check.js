async (page) => {
  const url = "http://127.0.0.1:4173/";
  const current = Math.floor(Date.now() / 1000);
  const bootstrap = {
    runtime: {
      platform_key: "test-platform-with-long-runtime-label",
      architecture: "test-architecture",
      devrelay_home: "/tmp/devrelay/very/long/runtime/home/path/used/to/exercise/scroll/behavior/without/frontend-fixtures",
      agent_socket_path: "/tmp/devrelay/agent/socket/path/with/a/long/name/devrelay-agent.sock",
      agent_socket_exists: true,
    },
    agent: {
      connected: true,
      socket_path: "/tmp/devrelay/agent/socket/path/with/a/long/name/devrelay-agent.sock",
      methods: [
        "rpc.negotiate",
        "agent.health",
        "status.get",
        "projects.list",
        "checkpoint.create",
        "diagnostics.export",
        "devices.list",
        "activity.list",
        "runs.list",
        "settings.get",
        "settings.update",
        "events.subscribe",
      ],
      health: { status: "ok" },
      errors: [],
    },
    projects: [
      {
        project_id: "project-alpha",
        display_name: "alpha-service-with-long-name-for-layout-verification",
        local_path: "/workspace/devrelay/test-fixtures/alpha-service-with-long-path-segment/and-another-segment/src",
        manifest_path: null,
        workspaces: {
          workspace_alpha: {
            workspace_id: "workspace-alpha",
            project_id: "project-alpha",
            device_id: "device-current",
            local_path: "/workspace/devrelay/test-fixtures/alpha-service-with-long-path-segment/and-another-segment/src",
            platform_profile: "test-platform",
            state: "active",
            last_seen_head: "abcdef1234567890abcdef",
            last_checkpoint_id: "checkpoint-alpha",
          },
        },
      },
      {
        project_id: "project-beta",
        display_name: "beta-library",
        local_path: "/workspace/devrelay/test-fixtures/beta-library",
        manifest_path: null,
        workspaces: {},
      },
    ],
    devices: [
      {
        device_id: "device-current",
        display_name: "current-device-with-long-name",
        platform_key: "test-os-current",
        architecture: "arm64",
        capabilities_json: "{}",
        paired_at_unix_seconds: current - 4000,
        last_seen_unix_seconds: current,
      },
      {
        device_id: "device-target",
        display_name: "target-device-with-long-name",
        platform_key: "test-os-target",
        architecture: "x86_64",
        capabilities_json: "{}",
        paired_at_unix_seconds: current - 3000,
        last_seen_unix_seconds: current - 60,
      },
    ],
    runs: [
      {
        task_run_id: "run-alpha-with-long-id-000000000000000000000001",
        project_id: "project-alpha",
        session_id: "session-alpha",
        state: "succeeded",
        command: "cargo test --workspace --all-features --locked --message-format=json",
        metadata: {
          result: "ok",
          artifact_path: "/workspace/devrelay/test-fixtures/output/artifacts/very/long/path/result.json",
        },
        created_at_unix_seconds: current - 120,
        updated_at_unix_seconds: current - 20,
      },
    ],
    activity: [
      {
        schema_version: 1,
        audit_id: 1,
        type: "checkpoint.created",
        outcome: "succeeded",
        summary: "Checkpoint verified for alpha-service-with-long-name-for-layout-verification",
        project_id: "project-alpha",
        detail: {
          path: "/workspace/devrelay/test-fixtures/alpha-service-with-long-path-segment/and-another-segment/src",
          note: "long metadata value used for overflow verification only",
        },
        created_at_unix_seconds: current - 8,
      },
    ],
    settings: {
      fabric_name: "Test fabric with long descriptive name",
      device_id: "device-current",
      device_name: "current-device-with-long-name",
      platform_key: "test-os-current",
      architecture: "arm64",
      resource_profile: "balanced",
      anchor_mode: "local-only",
      mdns_enabled: true,
      editor_command: "code",
      project_count: 2,
    },
  };

  await page.addInitScript((initialBootstrap) => {
    const calls = [];
    let latestBootstrap = structuredClone(initialBootstrap);
    window.__devrelayCalls = calls;
    window.__TAURI__ = {
      core: {
        invoke: async (name, args = {}) => {
          calls.push({ name, args });
          if (name === "ui_bootstrap") return structuredClone(latestBootstrap);
          if (name === "project_status") {
            return {
              ok: true,
              message: "status loaded",
              data: {
                status: {
                  head_oid: "abcdef1234567890abcdef",
                  branch: "feature/layout-verification-with-long-name",
                  upstream: "origin/feature/layout-verification-with-long-name",
                  counts: {
                    staged: 2,
                    unstaged: 3,
                    untracked: 4,
                    ignored: 0,
                    unmerged: 0,
                  },
                  clean: false,
                  initial: false,
                },
                entries: [],
                untracked: [],
              },
            };
          }
          if (name === "checkpoint_create") {
            return { ok: true, message: "checkpoint created", data: { checkpoint: { snapshot_id: "snapshot-alpha" } } };
          }
          if (name === "open_project") {
            return { ok: true, message: "project opened", data: "/workspace/devrelay/test-fixtures/alpha-service-with-long-path-segment" };
          }
          if (name === "diagnostics_export") {
            return { ok: true, message: "diagnostics exported", data: { path: "/tmp/devrelay/diagnostics/test-diagnostics.zip" } };
          }
          if (name === "settings_update") {
            latestBootstrap.settings = { ...latestBootstrap.settings, ...args.params };
            return { ok: true, message: "settings updated", data: { settings: latestBootstrap.settings } };
          }
          throw new Error(`unhandled command ${name}`);
        },
      },
      event: {
        listen: async () => () => {},
      },
    };
  }, bootstrap);

  const viewIds = ["continue", "projects", "devices", "runs", "activity", "settings"];
  const viewports = [
    { name: "desktop", width: 1280, height: 820 },
    { name: "short", width: 1024, height: 560 },
    { name: "narrow", width: 390, height: 740 },
  ];
  const reports = [];

  for (const viewport of viewports) {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await page.goto(url);
    await page.waitForSelector(".app-shell");
    await page.waitForTimeout(100);

    for (const view of viewIds) {
      await page.locator(`[data-view="${view}"]`).click();
      await page.waitForTimeout(80);
      const violations = await page.evaluate(() => {
        const scrollContainers = "[data-scroll-container], .main-scroll, .sidebar-scroll, .table-scroll, .panel-body.scroll, .json-box, .toast-region";
        const allowedOverflow = new Set(["auto", "scroll", "overlay"]);
        const nodes = Array.from(document.querySelectorAll("body *"));
        const problems = [];
        for (const node of nodes) {
          const rect = node.getBoundingClientRect();
          if (rect.width === 0 || rect.height === 0) continue;
          const style = getComputedStyle(node);
          const hasX = node.scrollWidth > node.clientWidth + 2;
          const hasY = node.scrollHeight > node.clientHeight + 2;
          if (!hasX && !hasY) continue;
          const inAllowedContainer = Boolean(node.closest(scrollContainers));
          const handlesX = allowedOverflow.has(style.overflowX) || allowedOverflow.has(style.overflow);
          const handlesY = allowedOverflow.has(style.overflowY) || allowedOverflow.has(style.overflow);
          if (!inAllowedContainer && ((hasX && !handlesX) || (hasY && !handlesY))) {
            problems.push({
              tag: node.tagName.toLowerCase(),
              className: String(node.className || ""),
              text: String(node.textContent || "").trim().slice(0, 80),
              scrollWidth: node.scrollWidth,
              clientWidth: node.clientWidth,
              scrollHeight: node.scrollHeight,
              clientHeight: node.clientHeight,
              overflowX: style.overflowX,
              overflowY: style.overflowY,
            });
          }
        }
        const pageOverflowX = document.documentElement.scrollWidth > window.innerWidth + 2;
        if (pageOverflowX) {
          problems.push({
            tag: "html",
            className: "page",
            text: "document horizontal overflow",
            scrollWidth: document.documentElement.scrollWidth,
            clientWidth: window.innerWidth,
            scrollHeight: document.documentElement.scrollHeight,
            clientHeight: window.innerHeight,
            overflowX: getComputedStyle(document.documentElement).overflowX,
            overflowY: getComputedStyle(document.documentElement).overflowY,
          });
        }
        return problems;
      });
      reports.push({ viewport: viewport.name, view, violations });
    }

    await page.screenshot({
      path: `output/playwright/devrelay-${viewport.name}.png`,
      fullPage: true,
    });
  }

  await page.locator('[data-view="continue"]').click();
  await page.locator('[data-action="checkpoint"]').click();
  await page.waitForTimeout(120);
  await page.locator('[data-action="open-project"]').click();
  await page.waitForTimeout(120);
  await page.locator('[data-action="diagnostics"]').click();
  await page.waitForTimeout(120);
  await page.locator('[data-view="settings"]').click();
  await page.locator('[name="editor_command"]').fill("system");
  await page.locator("[data-settings-form]").evaluate((form) => form.requestSubmit());
  await page.waitForTimeout(120);

  const calls = await page.evaluate(() => window.__devrelayCalls.map((call) => call.name));
  return { reports, calls };
}
