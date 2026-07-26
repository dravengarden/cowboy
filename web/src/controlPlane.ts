import { useSyncExternalStore } from "react";
import { persisted } from "./_store/mod.ts";
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
let activeSessionId = activeSessionStore.get();

export function getActiveSessionId(): string | null {
  return activeSessionId;
}

export function setActiveSessionId(id: string | null): void {
  if (id === activeSessionId) return;
  activeSessionId = id;
  activeSessionStore.set(id);
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
