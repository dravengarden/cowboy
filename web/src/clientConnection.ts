import { createConnectionStore } from "@cowboy/app-shell";
import { moduleEntryFromHtml } from "./serviceWorkerUpdates";
import { markClientReloadIntent, reportClientLog } from "./observability";
import {
  clientUpdateNavigationUrl,
  recordUpdateReload,
  updateReloadFailed,
} from "./updateReloadGuard";

// This module has no session/store dependency: update checks must not open a
// WebSocket or load the workspace while the product login gate is still up.
export function createClientConnection() {
  const loadedEntry = globalThis.document?.querySelector<HTMLScriptElement>(
    'script[type="module"][src]',
  )?.getAttribute("src");
  const origin = globalThis.location?.origin;
  const normalized = (entry: string): string => {
    const url = new URL(entry, origin);
    if (url.origin !== origin || !url.pathname.startsWith("/assets/")) {
      throw new Error("invalid deployed module entry");
    }
    return url.pathname + url.search;
  };
  const current = loadedEntry && origin && new URL(loadedEntry, origin).pathname.startsWith("/assets/")
    ? normalized(loadedEntry) : "development";
  let available: string | undefined;
  let downloaded: string | undefined;
  let navigating = false;
  const storage = () => {
    try { return globalThis.sessionStorage; } catch { return undefined; }
  };
  const store = createConnectionStore({
    versionUrl: "/version",
    updateProbe: async (trigger) => {
      if (current === "development") return { current };
      const response = await globalThis.fetch(`/?cowboy-bundle-probe=${Date.now()}`, {
        cache: "no-store",
        signal: AbortSignal.timeout(8_000),
      });
      if (!response.ok) throw new Error("deployed document unavailable");
      const entry = moduleEntryFromHtml(await response.text());
      if (!entry) throw new Error("deployed document has no module entry");
      const deployed = normalized(entry);
      const next = deployed === current ? undefined : deployed;
      if (next && next !== available) {
        reportClientLog("info", "client_update_available", "A new Cowboy client build is available", {
          loaded_entry: current.split("/").pop()!,
          deployed_entry: next.split("/").pop()!,
          trigger,
        });
      }
      available = next;
      return { current, available };
    },
  });
  const download = store.downloadUpdate;
  const reload = store.reloadIntoUpdate;
  store.downloadUpdate = async (progress, retry) => {
    downloaded = undefined;
    const target = available;
    if (!target) return "failed";
    if (!retry && updateReloadFailed(storage(), current, target)) {
      reportClientLog("warn", "client_update_no_progress", "The previous update kept the old client build; awaiting a retry", {
        loaded_entry: current.split("/").pop()!,
        deployed_entry: target.split("/").pop()!,
      });
      return "rejected";
    }
    const result = await download(progress, retry);
    if (target !== available) return "failed";
    if (result === "ready" || result === "unsupported") downloaded = target;
    return result;
  };
  store.reloadIntoUpdate = async (result) => {
    if (navigating || !downloaded || downloaded !== available || (result !== "ready" && result !== "unsupported")) return;
    navigating = true;
    recordUpdateReload(storage(), current, downloaded);
    markClientReloadIntent("deployed_version_detected", downloaded.split("/").pop());
    if (result === "ready") await reload(result);
    else globalThis.location.replace(clientUpdateNavigationUrl(globalThis.location.href, Date.now()));
  };
  return store;
}

export const conn = createClientConnection();
