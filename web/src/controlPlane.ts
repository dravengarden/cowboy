import { useSyncExternalStore } from "react";
import { persisted } from "./components/state/store/mod.ts";
import { conn, useStoreSelector } from "./store";
import {
  resolveWorkspaceBinding,
  type WorkspaceBinding,
} from "./workspaceBinding";
export type { WorkspaceBinding } from "./workspaceBinding";

// One lifecycle owner for every Cowboy product surface. The current daemon wire
// still carries Agent events alongside control state; Review must consume this
// singleton rather than create another socket/reconnect/update loop. The data
// planes can split behind this boundary without changing Mobile Shell.
export const controlPlaneConnection = conn;

const activeSessionStore = persisted<string | null>("cowboy:active-session", null, {
  serialize: (id) => id ?? "",
  deserialize: (raw) => (raw === "" ? null : raw),
});
const activeSessionListeners = new Set<() => void>();

function notificationSessionDeepLink(): string | null {
  try {
    const url = new URL(globalThis.location.href);
    const sessionId = url.searchParams.get("session");
    if (!sessionId || !/^[A-Za-z0-9_-]{1,160}$/.test(sessionId)) return null;
    url.searchParams.delete("session");
    globalThis.history.replaceState(globalThis.history.state, "", `${url.pathname}${url.search}${url.hash}`);
    return sessionId;
  } catch {
    return null;
  }
}

let activeSessionId = notificationSessionDeepLink() ?? activeSessionStore.get();
if (activeSessionId !== activeSessionStore.get()) activeSessionStore.set(activeSessionId);

function publishActiveSessionToWorker(): void {
  void globalThis.navigator?.serviceWorker?.ready.then((registration) => {
    (registration.active ?? registration.waiting)?.postMessage({
      type: "cowboy.active-session",
      sessionId: activeSessionId,
    });
  });
}

if (typeof globalThis.addEventListener === "function") {
  globalThis.addEventListener("focus", publishActiveSessionToWorker);
  globalThis.document?.addEventListener("visibilitychange", publishActiveSessionToWorker);
  publishActiveSessionToWorker();
}

export function getActiveSessionId(): string | null {
  return activeSessionId;
}

export function setActiveSessionId(id: string | null): void {
  if (id === activeSessionId) return;
  activeSessionId = id;
  activeSessionStore.set(id);
  publishActiveSessionToWorker();
  for (const listener of activeSessionListeners) listener();
}

function subscribeActiveSession(listener: () => void): () => void {
  activeSessionListeners.add(listener);
  return () => activeSessionListeners.delete(listener);
}

export function useActiveSessionId(): string | null {
  return useSyncExternalStore(
    subscribeActiveSession,
    getActiveSessionId,
    getActiveSessionId,
  );
}

export function useActiveWorkspaceBinding(): WorkspaceBinding | null {
  const selectedSessionId = useActiveSessionId();
  return useStoreSelector(
    (snapshot) => resolveWorkspaceBinding(snapshot.sessions, selectedSessionId),
    (previous, next) =>
      previous?.sessionId === next?.sessionId &&
      previous?.cwd === next?.cwd &&
      previous?.provider === next?.provider &&
      previous?.title === next?.title,
  );
}

/** A cheap invalidation signal for session-owned data planes.
 *
 * Review deliberately shares Agent's control-plane socket. File operations
 * performed by the active agent produce timeline events, so the latest sequence
 * is a useful prompt to revalidate Code's stable HTTP data plane without
 * creating another websocket. The data plane retains a slow polling fallback
 * for edits made outside Cowboy.
 */
export function useControlPlaneSessionActivity(
  sessionId: string | undefined,
): string {
  return useStoreSelector((snapshot) => {
    if (!sessionId) return "none";
    const timeline = snapshot.timelines.get(sessionId);
    return `${snapshot.connected ? "online" : "offline"}:${
      timeline?.at(-1)?.seq ?? 0
    }`;
  });
}
