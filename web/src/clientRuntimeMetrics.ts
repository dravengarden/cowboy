import { isNativeShell } from "./nativeShell";

const COMPOSER_DRAFT_PREFIX = "cowboy:composer-draft:";

export interface LocalStorageMetrics {
  bytes: number;
  entries: number;
  drafts: number;
}

export interface ClientRuntimeMetrics extends LocalStorageMetrics {
  storageUsageBytes?: number;
  storageQuotaBytes?: number;
  cacheBuckets?: number;
  jsHeapUsedBytes?: number;
  jsHeapLimitBytes?: number;
  surface: "Native app" | "Installed PWA" | "Browser tab";
  serviceWorker: "Active" | "Not controlling" | "Unsupported";
}

type StorageReader = Pick<Storage, "length" | "key" | "getItem">;

/** Count only data this origin persists synchronously on the current device. */
export function localStorageMetrics(
  storage: StorageReader | undefined,
): LocalStorageMetrics {
  if (!storage) return { bytes: 0, entries: 0, drafts: 0 };
  const encoder = new TextEncoder();
  let bytes = 0;
  let entries = 0;
  let drafts = 0;
  try {
    for (let index = 0; index < storage.length; index += 1) {
      const key = storage.key(index);
      if (key === null) continue;
      const value = storage.getItem(key) ?? "";
      bytes += encoder.encode(key).byteLength +
        encoder.encode(value).byteLength;
      entries += 1;
      if (key.startsWith(COMPOSER_DRAFT_PREFIX)) drafts += 1;
    }
  } catch {
    // A private or locked-down WebView may revoke Storage access mid-read.
  }
  return { bytes, entries, drafts };
}

function currentLocalStorage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

function finiteBytes(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : undefined;
}

export async function readClientRuntimeMetrics(): Promise<
  ClientRuntimeMetrics
> {
  const local = localStorageMetrics(currentLocalStorage());
  const navigator = globalThis.navigator;
  const estimate = (await navigator?.storage?.estimate().catch(() => ({})) ??
    {}) as StorageEstimate;
  const cacheBuckets = await globalThis.caches?.keys().then((keys) =>
    keys.length
  )
    .catch(() => undefined);
  const performanceMemory = (globalThis.performance as
    | Performance & {
      memory?: {
        usedJSHeapSize?: number;
        jsHeapSizeLimit?: number;
      };
    }
    | undefined)?.memory;
  const standalone =
    globalThis.matchMedia?.("(display-mode: standalone)").matches === true;
  const storageUsageBytes = finiteBytes(estimate.usage);
  // This is the browser's per-origin ceiling, not used or free device space.
  const storageQuotaBytes = finiteBytes(estimate.quota);
  // performance.memory is a non-standard Chromium extension. WebKit omits it,
  // so callers must omit this optional metric when the browser does not expose it.
  const jsHeapUsedBytes = finiteBytes(performanceMemory?.usedJSHeapSize);
  const jsHeapLimitBytes = finiteBytes(performanceMemory?.jsHeapSizeLimit);
  return {
    ...local,
    ...(storageUsageBytes !== undefined ? { storageUsageBytes } : {}),
    ...(storageQuotaBytes !== undefined ? { storageQuotaBytes } : {}),
    ...(cacheBuckets !== undefined ? { cacheBuckets } : {}),
    ...(jsHeapUsedBytes !== undefined ? { jsHeapUsedBytes } : {}),
    ...(jsHeapLimitBytes !== undefined ? { jsHeapLimitBytes } : {}),
    surface: isNativeShell()
      ? "Native app"
      : standalone
      ? "Installed PWA"
      : "Browser tab",
    serviceWorker: navigator === undefined || !("serviceWorker" in navigator)
      ? "Unsupported"
      : navigator.serviceWorker.controller
      ? "Active"
      : "Not controlling",
  };
}
