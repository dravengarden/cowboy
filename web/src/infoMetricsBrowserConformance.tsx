/** Full About view with synthetic transport; never reads production metrics. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { InfoContent } from "./InfoSheet";
import { ProductAuthGate } from "./auth/ProductAuthGate";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import type { MaintenanceMachine } from "./SessionMaintenancePanel";
import type { ServiceMetrics } from "./InfoRuntimeMetrics";

export async function runInfoMetricsBrowserConformance(
  keepFixture = false,
  screenshotWidth = 390,
): Promise<string[]> {
  const originalFetch = globalThis.fetch;
  const originalSocket = globalThis.WebSocket;
  const storageDescriptor = Object.getOwnPropertyDescriptor(
    navigator,
    "storage",
  );
  const container = document.createElement("div");
  container.style.cssText =
    "width:390px;max-width:100%;padding:16px;box-sizing:border-box;margin:auto";
  document.body.append(container);
  const renderErrors: string[] = [];
  const root = createRoot(container, {
    onUncaughtError: (error) => {
      renderErrors.push(String(error));
    },
  });
  const results: string[] = [];
  const requests: string[] = [];
  const now = Date.now();
  const gb = 1024 ** 3;
  let failMetrics = true;
  const service: ServiceMetrics = {
    db_bytes: 190.6 * 1024 ** 2,
    events_rows: 67_396,
    sessions_live: 110,
    sessions_deleted: 8,
    daemon_rss_bytes: 175.4 * 1024 ** 2,
    observability_pending: 0,
    observability_accepted_batches: 109,
    observability_dropped_batches: 0,
    observability_failed_log_batches: 3,
    observability_failed_metric_batches: 2,
  };
  const ovh: MaintenanceMachine = {
    machine: {
      id: "ovh",
      display_name: "OVH Matrix",
      connected: true,
      local: false,
      active_sessions: 12,
      capacity: { max_sessions: 40, draining: false },
      resources: {
        memory_total_bytes: 11.4 * gb,
        memory_available_bytes: 7.3 * gb,
        swap_total_bytes: 20 * gb,
        swap_free_bytes: 20 * gb - 122.1 * 1024 ** 2,
        agent_memory_bytes: 3.9 * gb,
        memory_headroom_bytes: 4.5 * gb,
        disk_total_bytes: 95.8 * gb,
        disk_available_bytes: 49.6 * gb,
        load_1m_milli: 300,
        cpu_count: 6,
        uptime_seconds: 17 * 86_400 + 4 * 3600,
        observed_at_ms: now - 4000,
      },
    },
    can_configure: true,
    policy: {
      memory_reclaim: true,
      reclaim_on_capacity: true,
      auto_update_idle_sessions: true,
    },
    sessions: 48,
    working: 3,
    warm: 9,
    sleeping: 36,
    starting: 0,
    updating: 1,
    waiting: [],
  };
  const older: MaintenanceMachine = {
    ...ovh,
    machine: {
      ...ovh.machine,
      id: "older",
      display_name: "MacBook Air",
      resources: null,
      active_sessions: 0,
      capacity: { max_sessions: 8, draining: false },
    },
    sessions: 0,
    working: 0,
    warm: 0,
    sleeping: 0,
    updating: 0,
  };
  const offline: MaintenanceMachine = {
    ...ovh,
    machine: {
      ...ovh.machine,
      id: "offline",
      display_name: "Disconnected Machine with a long descriptive name",
      connected: false,
      resources: { ...ovh.machine.resources!, observed_at_ms: now - 3_600_000 },
    },
    can_configure: false,
    waiting: [
      {
        id: "waiting",
        title: "Waiting task",
        reason: "waiting for Machine connection",
        paused: false,
      },
    ],
  };
  class FixtureSocket {
    static OPEN = 1;
    static CONNECTING = 0;
    static CLOSED = 3;
    readyState = 1;
    protocol = "cowboy-sync-v1";
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: string }) => void) | null = null;
    onclose: (() => void) | null = null;
    onerror = null;
    constructor() {
      setTimeout(() => {
        this.onopen?.();
        this.onmessage?.({
          data: JSON.stringify({ type: "sessions", sessions: [] }),
        });
        this.onmessage?.({
          data: JSON.stringify({ type: "bootstrap_complete" }),
        });
      }, 20);
    }
    send(): void {}
    close(): void {
      this.readyState = 3;
    }
  }
  bindProductSyncPrincipal("info-fixture");
  globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;
  Object.defineProperty(navigator, "storage", {
    configurable: true,
    value: {
      estimate: async () => ({ usage: 273.1 * 1024 ** 2, quota: 9.6 * gb }),
    },
  });
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input);
    if (!path.startsWith("/api/")) return originalFetch(input, init);
    requests.push(`${init?.method ?? "GET"} ${path}`);
    if (path === "/api/auth/status") {
      return Response.json({
        setup_required: false,
        registration: {
          enabled: false,
          mode: "disabled",
          accepts_registration: false,
        },
        providers: [],
        host_plugins: [],
        me: { account: "Info fixture", user_id: "info-fixture", role: "owner" },
      });
    }
    if (path === "/api/sync/dataset") {
      return Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"a".repeat(64)}`,
        user_id: "info-fixture",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      });
    }
    if (path === "/api/plugins") {
      return Response.json({
        providers: [],
        authentications: [],
        authentication_executors: [],
      });
    }
    if (path === "/api/metrics") {
      return failMetrics
        ? new Response("Metrics temporarily unavailable", { status: 503 })
        : Response.json(service);
    }
    if (path === "/api/session-maintenance") {
      return Response.json([older, offline, ovh]);
    }
    if (path === "/api/preferences/about") {
      return Response.json({
        preferences: {
          scope: "about",
          revision: 0,
          values: {
            usage_order: [],
            machine_order: ["ovh", "older", "offline"],
          },
        },
        choices: { usage_order: [], machine_order: [] },
        dataset: { dataset_id: "fixture", user_id: "info-fixture" },
      });
    }
    if (path === "/api/usage/executors") {
      return Response.json({ providers: {}, machines: [] });
    }
    if (path === "/api/usage") {
      return Response.json({
        refreshed_at_ms: now,
        next_refresh_at_ms: now + 60_000,
        refresh_interval_ms: 60_000,
        providers: [
          {
            provider: "Claude",
            status: "available",
            source: "Claude Code",
            observed_at_ms: now,
            rate_limits: {
              usageCredits: {
                enabled: false,
                balanceMinor: 0,
                balanceCurrency: "USD",
                spendCurrency: "USD",
                spentMinor: 1234,
                monthlyLimitMinor: 5000,
                autoReloadEnabled: false,
                expiryPolicyMonths: 12,
              },
              rateLimitResetCredits: {
                availableCount: 0,
                credits: [],
                grants: [{
                  title: "Weekly",
                  remaining: 2,
                  status: "paused",
                  expiresAt: (now + 86400000) / 1000,
                }],
              },
              rateLimits: {
                primary: {
                  usedPercent: 18,
                  windowDurationMins: 300,
                  resetsAt: (now + 3_600_000) / 1000,
                },
                secondary: {
                  usedPercent: 46,
                  windowDurationMins: 10_080,
                  resetsAt: (now + 86_400_000) / 1000,
                },
              },
            },
          },
          {
            provider: "Gemini",
            status: "unavailable",
            source: "Gemini ACP",
            observed_at_ms: now - 66_106 * 60_000,
            error: "Account quota is not exposed by this Provider.",
          },
        ],
      });
    }
    return new Response("Unavailable in isolated fixture", { status: 404 });
  }) as typeof fetch;
  const check = (ok: unknown, label: string) => {
    if (!ok) throw new Error(label);
    results.push(label);
  };
  const until = async (predicate: () => boolean) => {
    for (let n = 0; n < 150; n++) {
      if (predicate()) return;
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    throw new Error(
      `Info fixture timed out: ${container.textContent?.slice(0, 700)}; ${
        renderErrors.join(",")
      }; ${requests.join(",")}`,
    );
  };
  const render = (desktop = false) =>
    flushSync(() =>
      root.render(
        <SurfaceProvider>
          <BrowserProductTheme>
            <CssBaseline />
            <ProductAuthGate>
              <InfoContent desktop={desktop} />
            </ProductAuthGate>
          </BrowserProductTheme>
        </SurfaceProvider>,
      )
    );
  const machine = (id: string) =>
    container.querySelector<HTMLElement>(`[data-machine-resources="${id}"]`)!;
  try {
    render();
    await until(
      () => !!machine("ovh") && !!container.querySelector('[role="alert"]'),
    );
    check(
      container.textContent?.includes("HTTP 503"),
      "failed Service request shows a recoverable error",
    );
    check(
      !container.textContent?.includes("190.6 MB"),
      "failed metrics never render fabricated zero measurements",
    );
    const retry = [
      ...container.querySelectorAll<HTMLButtonElement>("button"),
    ].find((button) => button.textContent === "Retry")!;
    retry.focus();
    check(
      document.activeElement === retry,
      "metric recovery uses a keyboard reachable native button",
    );
    failMetrics = false;
    retry.click();
    await until(() => container.textContent?.includes("190.6 MB") === true);
    check(
      !container.querySelector('[role="alert"]'),
      "retry replaces the error with Service measurements",
    );
    check(
      machine("ovh")
        .querySelector('[aria-label="Memory"]')
        ?.getAttribute("aria-valuenow")
        ?.startsWith("35.9"),
      "host memory occupancy uses host capacity",
    );
    check(
      machine("ovh")
        .querySelector('[aria-label="AI workers"]')
        ?.getAttribute("aria-valuenow") === "30",
      "worker slots are independent of session history and memory",
    );
    check(
      machine("ovh")
        .querySelector('[role="img"]')
        ?.getAttribute("aria-label") === "3 working, 9 warm, 36 sleeping",
      "update overlays are not double counted in session distribution",
    );
    check(
      machine("ovh")
        .querySelector('[aria-label="Swap"]')
        ?.getAttribute("aria-valuetext")
        ?.includes("<1%"),
      "small nonzero swap usage remains distinguishable from zero",
    );
    check(
      machine("older").querySelectorAll('[role="meter"]').length === 1,
      "older Machines show known worker capacity without invented host readings",
    );
    check(
      machine("offline").textContent?.includes("not a live reading"),
      "disconnected host measurements are explicitly historical",
    );
    check(
      container
        .querySelector("[data-machine-resources]")
        ?.getAttribute("data-machine-resources") === "ovh",
      "saved Machine order survives the resource grid",
    );
    check(
      container.textContent?.includes("45d ago") &&
        container.textContent?.includes("NOT REPORTED"),
      "unavailable Provider data has readable age and no false waiting state",
    );
    check(
      [...container.querySelectorAll('[role="meter"]')].some(
        (meter) => meter.getAttribute("aria-valuetext") === "82% remaining",
      ),
      "quota meters expose remaining rather than consumed capacity",
    );
    for (const width of [320, 390, 768, 1200]) {
      container.style.width = `${width}px`;
      render(width >= 768);
      await new Promise((resolve) => requestAnimationFrame(resolve));
      check(
        container.scrollWidth <= container.clientWidth + 1 &&
          container.textContent?.includes("Auto reload") &&
          container.textContent?.includes("2 paused"),
        `About with credit details fits ${width}px without horizontal overflow`,
      );
    }
    const cards = [
      ...container.querySelectorAll<HTMLElement>("[data-machine-resources]"),
    ];
    check(
      cards[0]!.getBoundingClientRect().top ===
          cards[1]!.getBoundingClientRect().top &&
        container.querySelector('[data-storage-scope="service"]')!
            .getBoundingClientRect().top ===
          container.querySelector('[data-storage-scope="client"]')!
            .getBoundingClientRect().top,
      "wide About lays out storage and Machine cards side by side",
    );
    check(
      container.textContent?.includes("Cumulative since Service start") &&
        !container.textContent?.includes("Victoria failures"),
      "telemetry distinguishes historical counters from current delivery health",
    );
    check(
      requests.filter((request) => request === "GET /api/metrics").length ===
          2 && requests.every((request) => request.startsWith("GET ")),
      "inspection and viewport changes perform no mutations or extra metric polling",
    );
    if (keepFixture) {
      container.style.width = `${screenshotWidth}px`;
      render(screenshotWidth >= 768);
    }
    return results;
  } finally {
    if (!keepFixture) {
      root.unmount();
      container.remove();
      globalThis.fetch = originalFetch;
      globalThis.WebSocket = originalSocket;
      if (storageDescriptor) {
        Object.defineProperty(navigator, "storage", storageDescriptor);
      } else Reflect.deleteProperty(navigator, "storage");
    }
  }
}
