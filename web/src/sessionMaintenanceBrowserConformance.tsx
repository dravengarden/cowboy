/** Real maintenance UI; synthetic HTTP, no product credentials or mutations. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SessionMaintenancePanel, type MaintenanceMachine } from "./SessionMaintenancePanel";

export async function runSessionMaintenanceBrowserConformance(keepFixture = false): Promise<string[]> {
  const originalFetch = globalThis.fetch;
  const container = document.createElement("div");
  container.style.cssText = "width:360px;max-width:100%;padding:12px;box-sizing:border-box";
  document.body.append(container);
  const root = createRoot(container);
  const results: string[] = [];
  const row: MaintenanceMachine = {
    can_configure: true,
    machine: {
      id: "ovh", display_name: "OVH", capacity: { max_sessions: 8, draining: false },
      active_sessions: 8, local: false, connected: true,
      resources: {
        memory_total_bytes: 12 * 2 ** 30, memory_available_bytes: 8 * 2 ** 30,
        memory_headroom_bytes: 4 * 2 ** 30, agent_memory_bytes: 3 * 2 ** 30,
        swap_total_bytes: 20 * 2 ** 30, swap_free_bytes: 20 * 2 ** 30,
        load_1m_milli: 1200, cpu_count: 6, disk_total_bytes: 80 * 2 ** 30,
        disk_available_bytes: 50 * 2 ** 30, uptime_seconds: 7200, observed_at_ms: Date.now(),
      },
    },
    policy: { memory_reclaim: true, reclaim_on_capacity: true, auto_update_idle_sessions: true },
    sessions: 32, working: 2, warm: 6, sleeping: 24, starting: 0, updating: 0,
    waiting: [{ id: "waiting", title: "Pending task", reason: "all worker slots are occupied", paused: false }],
  };
  let reject = false;
  const writes: Record<string, boolean>[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    if (String(input) === "/api/session-maintenance") return Response.json([row]);
    if (String(input) === "/api/machines/ovh/maintenance" && init?.method === "PUT") {
      if (reject) return new Response("Maintenance unavailable", { status: 503 });
      const patch = JSON.parse(String(init.body)) as Record<string, boolean>;
      writes.push(patch);
      Object.assign(row.policy, patch);
      return Response.json(row.policy);
    }
    throw new Error("Unexpected maintenance request");
  }) as typeof fetch;
  const until = async (check: () => boolean): Promise<void> => {
    for (let n = 0; n < 100; n++) {
      if (check()) return;
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    throw new Error("Maintenance fixture timed out");
  };
  const check = (value: boolean, name: string): void => {
    if (!value) throw new Error(name);
    results.push(name);
  };
  try {
    flushSync(() => root.render(<BrowserProductTheme><CssBaseline /><SessionMaintenancePanel /></BrowserProductTheme>));
    await until(() => container.querySelectorAll('input[type="checkbox"]').length === 3);
    const text = container.textContent ?? "";
    check(text.includes("32 sessions") && text.includes("24 sleeping") && text.includes("all worker slots"),
      "record counts and durable resource wait are distinct from worker capacity");
    check(container.scrollWidth <= container.clientWidth, "phone width has no horizontal overflow");
    const inputs = () => [...container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
    for (const [index, key] of ["memory_reclaim", "reclaim_on_capacity", "auto_update_idle_sessions"].entries()) {
      inputs()[index]!.click();
      await until(() => writes.length === index + 1 && !inputs()[index]!.disabled && !inputs()[index]!.checked);
      check(Object.keys(writes[index]!).length === 1 && writes[index]![key] === false,
        `independent durable policy: ${key}`);
    }
    reject = true;
    inputs()[0]!.click();
    await until(() => container.querySelector('[role="alert"]') !== null && !inputs()[0]!.disabled);
    check(!inputs()[0]!.checked, "failed save is visible and preserves the accepted setting");
    container.style.width = "960px";
    check(container.scrollWidth <= container.clientWidth, "desktop width has no horizontal overflow");
    row.can_configure = false;
    flushSync(() => root.render(<BrowserProductTheme><CssBaseline /><SessionMaintenancePanel key="viewer" /></BrowserProductTheme>));
    await until(() => inputs().length === 3);
    check(inputs().every((input) => input.disabled), "viewer can inspect resources without unauthorized controls");
    if (keepFixture) {
      reject = false;
      row.can_configure = true;
      flushSync(() => root.render(<BrowserProductTheme><CssBaseline /><SessionMaintenancePanel key="operator" /></BrowserProductTheme>));
      await until(() => inputs().length === 3);
      inputs()[0]!.click();
      await until(() => container.querySelector('[role="alert"]') === null && !inputs()[0]!.disabled);
    }
    return results;
  } finally {
    if (!keepFixture) { root.unmount(); container.remove(); }
    globalThis.fetch = originalFetch;
  }
}
