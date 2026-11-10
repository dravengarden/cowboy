import type { MachineBrowserResources, MachineSummary } from "./protocol";

/** Resource polling does not transfer Plugin or workspace inventories. */
export type MachineResourceSummary = Pick<MachineSummary,
  "id" | "display_name" | "local" | "connected" | "capacity" | "active_sessions"
> & { resources?: MachineSummary["resources"] | null };

export function formatBytes(n: number): string {
  if (n < 1024) return `${String(n)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(1)} ${units[i] ?? "B"}`;
}

function formatUsage(used: number, total: number): string {
  return `${formatBytes(used)} / ${formatBytes(total)}`;
}

function formatAge(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 60) return `${String(seconds)}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${String(minutes)}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${String(hours)}h ago`;
  return `${String(Math.round(hours / 24))}d ago`;
}

export function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  if (days > 0) return `${String(days)}d ${String(hours)}h`;
  return `${String(hours)}h ${String(Math.floor((seconds % 3_600) / 60))}m`;
}

/** Remote Machines first, then by name; the Controller's own Machine last. */
export function machinesForResources(
  machines: readonly MachineSummary[],
): MachineSummary[] {
  return [...machines].sort((a, b) =>
    Number(a.local) - Number(b.local) ||
    a.display_name.localeCompare(b.display_name)
  );
}

/** Below this, retired Plugin generations or other owners' data need attention. */
const LOW_DISK_BYTES = 15 * 1024 ** 3;

/** One line under the Machine name: placement, reachability and freshness. */
export function machineResourcesCaption(
  machine: MachineResourceSummary,
  nowMs: number,
): string {
  const placement = machine.local ? "Controller host" : "Remote";
  const observed = machine.resources
    ? ` · updated ${formatAge(nowMs - machine.resources.observed_at_ms)}`
    : "";
  if (!machine.connected) return `${placement} · offline${observed}`;
  if (!machine.resources) {
    return `${placement} · resource reporting needs a newer Machine`;
  }
  const lowDisk = machine.resources.disk_available_bytes < LOW_DISK_BYTES
    ? ` · low disk: ${formatBytes(machine.resources.disk_available_bytes)} free`
    : "";
  return `${placement}${observed}${lowDisk}`;
}

/** Label/value tiles for one Machine; sessions are always known. */
export function machineResourceMetrics(
  machine: MachineResourceSummary,
): [string, string][] {
  const sessions: [string, string] = [
    "AI workers",
    `${String(machine.active_sessions)} / ${String(machine.capacity.max_sessions)}`,
  ];
  const resources = machine.resources;
  if (!resources) return [sessions];
  const metrics: [string, string][] = [
    [
      "Memory",
      formatUsage(
        resources.memory_total_bytes - resources.memory_available_bytes,
        resources.memory_total_bytes,
      ),
    ],
    [
      "Swap",
      resources.swap_total_bytes > 0
        ? formatUsage(
          resources.swap_total_bytes - resources.swap_free_bytes,
          resources.swap_total_bytes,
        )
        : "None",
    ],
    sessions,
  ];
  if (resources.agent_memory_bytes !== undefined) {
    metrics.push(["Agent memory", formatBytes(resources.agent_memory_bytes)]);
  }
  if (resources.memory_headroom_bytes !== undefined) {
    metrics.push(["Worker budget (estimate)", formatBytes(resources.memory_headroom_bytes)]);
  }
  if (resources.browser) metrics.push(...browserResourceMetrics(resources.browser));
  metrics.push(
    [
      "Load (1 min)",
      `${(resources.load_1m_milli / 1000).toFixed(2)} · ${
        String(resources.cpu_count)
      } CPU${resources.cpu_count === 1 ? "" : "s"}`,
    ],
    [
      "Disk",
      formatUsage(
        resources.disk_total_bytes - resources.disk_available_bytes,
        resources.disk_total_bytes,
      ),
    ],
    ["Uptime", formatUptime(resources.uptime_seconds)],
  );
  return metrics;
}

/** Slots the next open is admitted against; older Machines report only
 * registered instances, which may undercount starts and retained units. */
export function browserOccupiedSlots(browser: MachineBrowserResources): number {
  return Math.max(browser.running, browser.occupied_slots ?? browser.running);
}

/** Browser instance slots: running instances, and free slots by what
 * admission holds rather than by what is running. */
export function browserSlotsDetail(browser: MachineBrowserResources): string {
  if (browser.max_instances <= 0) return "Disabled";
  const free = Math.max(0, browser.max_instances - browserOccupiedSlots(browser));
  if (free === 0) return "All slots in use";
  return `${String(browser.running)} running · ${String(free)} free`;
}

/** Label/value pairs for a Machine that reports browser usage. */
export function browserResourceMetrics(
  browser: MachineBrowserResources,
): [string, string][] {
  return [
    [
      "Browsers",
      `${String(browser.running)} / ${String(browser.max_instances)} running`,
    ],
    [
      "Browser memory",
      browser.running === 0
        ? "None running"
        : `${formatBytes(browser.memory_bytes)} (budget ${formatBytes(browser.budget_bytes)})`,
    ],
  ];
}

/** Browser units run in the Agent worker slice; say what that means for the
 * slice figures the card shows beside them. */
export const BROWSER_MEMORY_NOTE =
  "Browser memory is part of Agent memory and is already deducted from Headroom.";
