import { useEffect, useMemo, useSyncExternalStore } from "react";
import { productSyncDatabase } from "../productSyncDatabase";
import { createDraftRepository } from "./repository";
import { documentNotice } from "./DocumentNotifications";

let repository: ReturnType<typeof createDraftRepository> | undefined;
/** The controller's `drafts` push; inert until this device opens Drafts. */
export function announceDraftChanges(value: unknown): void {
  repository?.announce(value);
}
export function draftRepository(): ReturnType<typeof createDraftRepository> {
  return repository ??= createDraftRepository({
    persistence: (id) =>
      productSyncDatabase.outbox({
        kind: "document",
        document: id,
        state: "entry",
      }),
    cache: productSyncDatabase.cache({ kind: "service", state: "drafts" }),
    localIds: () => productSyncDatabase.draftDocumentIds(),
    signal: productSyncDatabase.signal,
    notify: (message) => documentNotice(message),
    request: async (path, init = {}) => {
      const dataset = await productSyncDatabase.ready();
      if (productSyncDatabase.signal.aborted) {
        throw new Error("Draft account was closed");
      }
      return fetch(path, {
        ...init,
        credentials: "same-origin",
        cache: "no-store",
        signal: AbortSignal.any([
          productSyncDatabase.signal,
          AbortSignal.timeout(12000),
        ]),
        headers: {
          "Content-Type": "application/json",
          "x-cowboy-dataset": dataset.dataset_id,
        },
      });
    },
  });
}

export function useDraftLibrary() {
  const owner = draftRepository();
  useEffect(() => {
    void owner.start();
    const refresh = (): void => {
      if (document.visibilityState !== "hidden") void owner.refresh();
    };
    const interval = globalThis.setInterval(refresh, 15000);
    globalThis.addEventListener("online", refresh);
    globalThis.addEventListener("focus", refresh);
    return () => {
      globalThis.clearInterval(interval);
      globalThis.removeEventListener("online", refresh);
      globalThis.removeEventListener("focus", refresh);
    };
  }, [owner]);
  return useSyncExternalStore(owner.subscribe, owner.get, owner.get);
}

export function useDraftDocument(id: string) {
  const owner = useMemo(() => draftRepository().document(id), [id]);
  useEffect(() => {
    void owner.refresh();
    const refresh = (): void => {
      if (document.visibilityState !== "hidden") void owner.refresh();
    };
    const interval = globalThis.setInterval(refresh, 10000);
    globalThis.addEventListener("focus", refresh);
    return () => {
      globalThis.clearInterval(interval);
      globalThis.removeEventListener("focus", refresh);
    };
  }, [owner]);
  return useSyncExternalStore(owner.subscribe, owner.get, owner.get);
}
