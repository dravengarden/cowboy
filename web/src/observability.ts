import { newUuid } from "./uuid";
import { cleanTelemetryAttributes, cleanTelemetryMessage, retryTelemetryStatus, TelemetryQueue } from "./telemetryQueue.ts";

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

export const CRASH_INCIDENT_SEVERITY = "critical" as const;

const queue = new TelemetryQueue();
let installed = false;
let flushing = false;
let activeRequest: AbortController | null = null;
let context: { session_id?: string; machine_id?: string; trace_id?: string } = {};
const RELOAD_INTENT_KEY = "cowboy:observability-reload-intent";

function newId(): string {
  return newUuid();
}

function stableClientId(): string {
  const key = "cowboy:observability-client-id";
  try {
    const existing = globalThis.localStorage.getItem(key);
    if (existing && /^[a-zA-Z0-9_.:-]{1,128}$/.test(existing)) return existing;
    const created = newId();
    globalThis.localStorage.setItem(key, created);
    return created;
  } catch {
    return `ephemeral-${newId()}`;
  }
}

const clientId = stableClientId();

function buildIdentity(): string {
  return globalThis.document?.querySelector<HTMLScriptElement>('script[type="module"][src]')
    ?.src.split("/").pop()?.slice(0, 128) ?? "development";
}

function cleanMessage(value: unknown): string {
  return cleanTelemetryMessage(value);
}

export function setObservabilityContext(next: typeof context): void {
  context = Object.fromEntries(Object.entries(next).filter(([key, value]) => ["session_id", "machine_id", "trace_id"].includes(key) && typeof value === "string" && /^[a-zA-Z0-9_.:-]{1,128}$/.test(value)));
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
  if (!/^[a-zA-Z0-9_.:-]{1,64}$/.test(eventName)) return;
  queue.capture("logs", {
    occurred_at_ms: Date.now(),
    level,
    event_name: eventName,
    message: cleanMessage(message),
    attributes: cleanTelemetryAttributes(attributes),
  } satisfies PendingLog, context);
  if (level === "error" || queue.size >= 50) {
    void flushObservability();
  }
}

export function reportClientMetric(
  name: string,
  value: number,
  dimensions: Record<string, string> = {},
): void {
  if (!Number.isFinite(value) || !/^[a-zA-Z0-9_]{1,64}$/.test(name)) return;
  const labels = Object.fromEntries(Object.entries(dimensions).filter(([key, value]) =>
    ["connection", "transport", "reason"].includes(key) && /^[a-zA-Z0-9_-]{1,64}$/.test(value)
  ));
  queue.capture("metrics", { occurred_at_ms: Date.now(), name, value, dimensions: labels } satisfies PendingMetric, context);
}

export function reportClientIncident(
  classification: string,
  severity: PendingIncident["severity"],
  summary: unknown,
  detail: Record<string, Scalar> = {},
): void {
  if (!/^[a-zA-Z0-9_.:-]{1,64}$/.test(classification)) return;
  queue.capture("incidents", {
    id: newId(),
    occurred_at_ms: Date.now(),
    classification,
    severity,
    summary: cleanMessage(summary),
    detail: cleanTelemetryAttributes(detail),
  } satisfies PendingIncident, context);
  void flushObservability();
}

function takeBatch() {
  const ua = globalThis.navigator?.userAgent ?? "";
  return queue.take({
      id: clientId,
      platform: /iPad|iPhone|iPod/.test(ua) ? "ios" : /Macintosh/.test(ua) ? "macos" : "web",
      app_version: buildIdentity(),
      surface: globalThis.matchMedia?.("(pointer: coarse)").matches ? "mobile" : "desktop",
  }, newId);
}

export async function flushObservability(): Promise<void> {
  if (flushing) return;
  const batch = takeBatch();
  if (!batch) return;
  flushing = true;
  const controller = new AbortController();
  activeRequest = controller;
  const timeout = globalThis.setTimeout(() => controller.abort(), 8000);
  try {
    const response = await globalThis.fetch("/api/observability/batches", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: batch.body,
      signal: controller.signal,
    });
    queue.settle(batch, !response.ok && retryTelemetryStatus(response.status));
  } catch {
    queue.settle(batch, true);
  } finally {
    flushing = false;
    activeRequest = null;
    globalThis.clearTimeout(timeout);
  }
}

function beaconFlush(): void {
  if (flushing) return;
  const batch = takeBatch();
  if (!batch) return;
  try {
    const accepted = globalThis.navigator.sendBeacon(
      "/api/observability/batches",
      new Blob([batch.body], { type: "application/json" }),
    );
    queue.settle(batch, !accepted);
  } catch {
    queue.settle(batch, true);
  }
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
    // Storage can itself throw; diagnostics must never break startup.
  }
}

async function reportRuntimeIdentity(): Promise<void> {
  const navigation = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined;
  const nativeRoot = globalThis as typeof globalThis & {
    __cowboyNativeShell?: unknown;
    __cowboyReadClipboard?: unknown;
    __cowboyClipboardImageStatus?: unknown;
    __cowboyReadClipboardImages?: unknown;
  };
  const attributes: Record<string, Scalar> = {
    navigation_type: navigation?.type ?? "unknown",
    native_shell: nativeRoot.__cowboyNativeShell === true,
    native_text_read_bridge: typeof nativeRoot.__cowboyReadClipboard === "function",
    native_image_status_bridge: typeof nativeRoot.__cowboyClipboardImageStatus === "function",
    native_image_read_bridge: typeof nativeRoot.__cowboyReadClipboardImages === "function",
  };
  try {
    const [versionResponse, workerResponse] = await Promise.all([
      globalThis.fetch("/version", { cache: "no-store", signal: AbortSignal.timeout(8000) }),
      globalThis.fetch("/sw.js", { cache: "no-store", signal: AbortSignal.timeout(8000) }),
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
  globalThis.addEventListener("cowboy:product-sign-out", () => {
    activeRequest?.abort();
    queue.clear();
    context = {};
  });
  reportReloadCompletion();
  void reportRuntimeIdentity();
  globalThis.addEventListener("error", (event) => {
    const detail = {
      filename: event.filename?.split("/").pop() ?? "",
      line: event.lineno,
      column: event.colno,
    };
    reportClientLog("error", "window_error", event.error ?? event.message, detail);
    reportClientIncident(
      "client_window_error",
      CRASH_INCIDENT_SEVERITY,
      event.error ?? event.message,
      detail,
    );
  });
  globalThis.addEventListener("unhandledrejection", (event) => {
    reportClientLog("error", "unhandled_rejection", event.reason);
    reportClientIncident("client_unhandled_rejection", CRASH_INCIDENT_SEVERITY, event.reason);
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
