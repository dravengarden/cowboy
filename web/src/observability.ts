import { newUuid } from "./uuid";

type LogLevel = "debug" | "info" | "warn" | "error";
type Scalar = string | number | boolean | null;

interface PendingLog {
  occurred_at_ms: number;
  level: LogLevel;
  event_name: string;
  message: string;
  attributes: Record<string, Scalar>;
}

interface PendingMetric {
  occurred_at_ms: number;
  name: string;
  value: number;
  dimensions: Record<string, string>;
}

interface PendingIncident {
  id: string;
  occurred_at_ms: number;
  classification: string;
  severity: "warning" | "error" | "critical";
  summary: string;
  detail: Record<string, Scalar>;
}

const logs: PendingLog[] = [];
const metrics: PendingMetric[] = [];
const incidents: PendingIncident[] = [];
let installed = false;
let flushing = false;
let context: { session_id?: string; machine_id?: string; trace_id?: string } = {};
const RELOAD_INTENT_KEY = "cowboy:observability-reload-intent";

function newId(): string {
  return newUuid();
}

function stableClientId(): string {
  const key = "cowboy:observability-client-id";
  try {
    const existing = globalThis.localStorage.getItem(key);
    if (existing) return existing;
    const created = newId();
    globalThis.localStorage.setItem(key, created);
    return created;
  } catch {
    return `ephemeral-${newId()}`;
  }
}

const clientId = stableClientId();

function buildIdentity(): string {
  return globalThis.document.querySelector<HTMLScriptElement>('script[type="module"][src]')
    ?.src.split("/").pop()?.slice(0, 128) ?? "development";
}

function cleanMessage(value: unknown): string {
  const text = value instanceof Error ? value.message : String(value ?? "Unknown error");
  return text
    .replace(/(authorization|token|secret|password|cookie)=?[^\s,;]*/gi, "$1=[redacted]")
    .slice(0, 4096);
}

function trimPending(): void {
  while (logs.length + metrics.length + incidents.length > 200) {
    const debug = logs.findIndex((entry) => entry.level === "debug");
    if (debug >= 0) logs.splice(debug, 1);
    else if (metrics.length > 0) metrics.shift();
    else if (logs.length > 0) logs.shift();
    else break;
  }
}

export function setObservabilityContext(next: typeof context): void {
  context = { ...next };
}

export function markClientReloadIntent(reason: string, targetBuild?: string): void {
  try {
    globalThis.sessionStorage.setItem(RELOAD_INTENT_KEY, JSON.stringify({
      reason,
      from_build: buildIdentity(),
      target_build: targetBuild ?? "unknown",
      marked_at_ms: Date.now(),
    }));
  } catch {
    // Reload still proceeds when WebKit denies session storage.
  }
  reportClientLog("info", "client_reload_planned", "Cowboy client reload planned", {
    reason,
    target_build: targetBuild ?? "unknown",
  });
  void flushObservability();
}

export function reportClientLog(
  level: LogLevel,
  eventName: string,
  message: unknown,
  attributes: Record<string, Scalar> = {},
): void {
  logs.push({
    occurred_at_ms: Date.now(),
    level,
    event_name: eventName,
    message: cleanMessage(message),
    attributes,
  });
  trimPending();
  if (level === "error" || logs.length + metrics.length + incidents.length >= 50) {
    void flushObservability();
  }
}

export function reportClientMetric(
  name: string,
  value: number,
  dimensions: Record<string, string> = {},
): void {
  if (!Number.isFinite(value)) return;
  metrics.push({ occurred_at_ms: Date.now(), name, value, dimensions });
  trimPending();
}

export function reportClientIncident(
  classification: string,
  severity: PendingIncident["severity"],
  summary: unknown,
  detail: Record<string, Scalar> = {},
): void {
  incidents.push({
    id: newId(),
    occurred_at_ms: Date.now(),
    classification,
    severity,
    summary: cleanMessage(summary),
    detail,
  });
  trimPending();
  void flushObservability();
}

function takeBatch(): Record<string, unknown> | null {
  if (logs.length === 0 && metrics.length === 0 && incidents.length === 0) return null;
  const ua = globalThis.navigator.userAgent;
  return {
    batch_id: newId(),
    client: {
      id: clientId,
      platform: /iPad|iPhone|iPod/.test(ua) ? "ios" : /Macintosh/.test(ua) ? "macos" : "web",
      app_version: buildIdentity(),
      surface: globalThis.matchMedia("(pointer: coarse)").matches ? "mobile" : "desktop",
    },
    context,
    logs: logs.splice(0),
    metrics: metrics.splice(0),
    incidents: incidents.splice(0),
  };
}

function restoreBatch(batch: Record<string, unknown>): void {
  logs.unshift(...(batch.logs as PendingLog[]));
  metrics.unshift(...(batch.metrics as PendingMetric[]));
  incidents.unshift(...(batch.incidents as PendingIncident[]));
  trimPending();
}

export async function flushObservability(): Promise<void> {
  if (flushing) return;
  const batch = takeBatch();
  if (!batch) return;
  flushing = true;
  try {
    const response = await globalThis.fetch("/api/observability/batches", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(batch),
      keepalive: true,
    });
    if (!response.ok) restoreBatch(batch);
  } catch {
    restoreBatch(batch);
  } finally {
    flushing = false;
  }
}

function beaconFlush(): void {
  const batch = takeBatch();
  if (!batch) return;
  const accepted = globalThis.navigator.sendBeacon(
    "/api/observability/batches",
    new Blob([JSON.stringify(batch)], { type: "application/json" }),
  );
  if (!accepted) restoreBatch(batch);
}

function installPerformanceObservers(): void {
  if (!("PerformanceObserver" in globalThis)) return;
  try {
    const observer = new PerformanceObserver((list) => {
      const entries = list.getEntries();
      if (entries.length === 0) return;
      reportClientMetric("long_task_count", entries.length);
      reportClientMetric("long_task_duration_ms_sum", entries.reduce((sum, item) => sum + item.duration, 0));
      reportClientMetric("long_task_duration_ms_max", Math.max(...entries.map((item) => item.duration)));
    });
    observer.observe({ type: "longtask", buffered: true });
  } catch {
    // Older WebKit does not expose long-task entries.
  }
}

function reportReloadCompletion(): void {
  try {
    const raw = globalThis.sessionStorage.getItem(RELOAD_INTENT_KEY);
    if (!raw) return;
    globalThis.sessionStorage.removeItem(RELOAD_INTENT_KEY);
    const value = JSON.parse(raw) as Record<string, unknown>;
    const markedAt = typeof value.marked_at_ms === "number" ? value.marked_at_ms : Date.now();
    reportClientLog("info", "client_reload_completed", "Cowboy client reload completed", {
      reason: typeof value.reason === "string" ? value.reason : "unknown",
      from_build: typeof value.from_build === "string" ? value.from_build : "unknown",
      target_build: typeof value.target_build === "string" ? value.target_build : "unknown",
      reload_duration_ms: Math.max(0, Date.now() - markedAt),
    });
  } catch {
    globalThis.sessionStorage.removeItem(RELOAD_INTENT_KEY);
  }
}

async function reportRuntimeIdentity(): Promise<void> {
  const navigation = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined;
  const attributes: Record<string, Scalar> = {
    navigation_type: navigation?.type ?? "unknown",
  };
  try {
    const [versionResponse, workerResponse] = await Promise.all([
      globalThis.fetch("/version", { cache: "no-store" }),
      globalThis.fetch("/sw.js", { cache: "no-store" }),
    ]);
    if (versionResponse.ok) {
      const value = await versionResponse.json() as { version?: unknown };
      if (typeof value.version === "string") attributes.server_version = value.version.slice(0, 128);
    }
    if (workerResponse.ok) {
      const version = /const VERSION = ["']([^"']+)/.exec(await workerResponse.text())?.[1];
      if (version) attributes.service_worker_version = version.slice(0, 128);
    }
  } catch {
    attributes.identity_probe = "unavailable";
  }
  reportClientLog("info", "client_runtime_identity", "Cowboy client runtime identity", attributes);
}

export function installObservability(): void {
  if (installed) return;
  installed = true;
  reportReloadCompletion();
  void reportRuntimeIdentity();
  globalThis.addEventListener("error", (event) => {
    const detail = {
      filename: event.filename?.split("/").pop() ?? "",
      line: event.lineno,
      column: event.colno,
    };
    reportClientLog("error", "window_error", event.error ?? event.message, detail);
  });
  globalThis.addEventListener("unhandledrejection", (event) => {
    reportClientLog("error", "unhandled_rejection", event.reason);
    reportClientIncident("client_unhandled_rejection", "error", event.reason);
  });
  globalThis.addEventListener("online", () => {
    reportClientLog("info", "network_online", "Browser network became available");
    void flushObservability();
  });
  globalThis.addEventListener("offline", () => {
    reportClientLog("warn", "network_offline", "Browser network became unavailable");
  });
  globalThis.addEventListener("pagehide", beaconFlush);
  globalThis.addEventListener("load", () => {
    const navigation = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined;
    if (navigation) reportClientMetric("navigation_duration_ms", navigation.duration);
  }, { once: true });
  globalThis.setInterval(() => void flushObservability(), 30_000);
  installPerformanceObservers();
}
