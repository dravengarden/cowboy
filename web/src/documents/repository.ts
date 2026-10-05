import {
  type ClientSnapshot,
  type LocalPersistence,
  type Mutation,
  replicatedStore,
  snapshotPatch,
} from "@cowboy/state-sync";
import type { ProductCache } from "../productSyncDatabase";
import {
  decodeDraft,
  type DraftChange,
  type DraftDocument,
  type DraftMetadata,
  draftMetadata,
  type DraftMutationArgs,
  draftMutators,
  expectedRevision,
} from "./model";

export interface DraftDocumentSnapshot {
  readonly document: DraftDocument | null;
  readonly phase:
    | "loading"
    | "saved"
    | "local"
    | "saving"
    | "conflict"
    | "error";
  readonly error: string | null;
  readonly remote: DraftDocument | null;
}
export interface DraftLibrarySnapshot {
  readonly entries: readonly DraftMetadata[];
  readonly loaded: boolean;
  readonly error: string | null;
}
interface RepositoryOptions {
  persistence(
    id: string,
  ): LocalPersistence<ClientSnapshot<DraftDocument | null>>;
  cache: ProductCache<readonly DraftMetadata[]>;
  localIds(): Promise<string[]>;
  request(path: string, init?: RequestInit): Promise<Response>;
  signal: AbortSignal;
}

/** Per-document durable replicas keep large bodies out of the library index.
 * Outbox delta merges preserve other tabs' authored operations. The server
 * arbitrates versions; conflicts leave the entire local branch recoverable. */
export function createDraftRepository(options: RepositoryOptions) {
  const owners = new Map<string, ReturnType<typeof makeOwner>>();
  const listeners = new Set<() => void>();
  let library: DraftLibrarySnapshot = {
    entries: [],
    loaded: false,
    error: null,
  };
  let starting: Promise<void> | undefined;
  let refreshing: Promise<void> | undefined;
  let stopped = false;
  function assertActive(): void {
    if (stopped || options.signal.aborted) {
      throw new Error("Draft account was closed");
    }
  }
  function emit(): void {
    if (!stopped) { for (const listener of listeners) listener(); }
  }
  function updateIndex(document: DraftDocument | null): void {
    if (!document || stopped) return;
    const metadata = draftMetadata(document);
    const existing = library.entries.find((d) => d.id === document.id);
    if (existing && existing.revision > document.revision) return;
    if (existing && JSON.stringify(existing) === JSON.stringify(metadata)) {
      return;
    }
    library = {
      ...library,
      entries: [
        ...library.entries.filter((d) => d.id !== document.id),
        metadata,
      ],
    };
    emit();
  }
  function makeOwner(id: string) {
    let snapshot: DraftDocumentSnapshot = {
      document: null,
      phase: "loading",
      error: null,
      remote: null,
    };
    const subscribers = new Set<() => void>();
    let hydrated: Promise<void> | undefined;
    let fetching: Promise<void> | undefined;
    let draining = false;
    let sealed = false;
    const deliverable = new Map<string, Mutation>();
    const replica = replicatedStore<DraftDocument | null, typeof draftMutators>(
      {
        clientId: `draft-${crypto.randomUUID()}`,
        initial: null,
        mutators: draftMutators,
        local: options.persistence(id),
        send: (mutation) => {
          deliverable.set(mutation.id, mutation);
          void drain();
        },
        onChange: () => publish(),
      },
    );
    function publish(patch: Partial<DraftDocumentSnapshot> = {}): void {
      if (stopped || sealed) return;
      snapshot = { ...snapshot, document: replica.get(), ...patch };
      updateIndex(snapshot.document);
      for (const subscriber of subscribers) subscriber();
    }
    function hydrate(): Promise<void> {
      return hydrated ??= replica.hydrate().then(() => {
        assertActive();
        publish({ phase: replica.pending().length ? "local" : "saved" });
        replica.resend();
      });
    }
    async function drain(): Promise<void> {
      if (draining || stopped || sealed || snapshot.phase === "conflict") {
        return;
      }
      draining = true;
      try {
        for (;;) {
          const mutation = replica.pending().find((m) => deliverable.has(m.id));
          if (!mutation || stopped || sealed) break;
          const args = mutation.args as DraftMutationArgs;
          publish({ phase: "saving", error: null });
          let response: Response;
          try {
            response = await options.request("/api/drafts/mutations", {
              method: "POST",
              body: JSON.stringify({
                operation_id: mutation.id,
                document_id: id,
                expected_revision: args.expected_revision,
                change: args.change,
              }),
            });
          } catch (error) {
            publish({
              phase: "local",
              error: error instanceof Error
                ? error.message
                : "Waiting for connection",
            });
            break;
          }
          if (stopped || sealed) break;
          const result: unknown = await response.json();
          if (!response.ok) {
            const detail = result as { error?: string; current?: unknown };
            if (response.status === 409 && "current" in detail) {
              const remote = detail.current
                ? decodeDraft(detail.current)
                : null;
              if (["move", "trash", "restore"].includes(args.change.type)) {
                await replica.confirmDurably([mutation.id]);
                deliverable.delete(mutation.id);
                replica.applyPatch(
                  snapshotPatch(remote?.revision ?? 0, remote, []),
                  { force: true },
                );
                publish({
                  phase: "error",
                  error:
                    "The draft changed elsewhere. This location change was not applied.",
                  remote,
                });
              } else {publish({
                  phase: "conflict",
                  error: detail.error ?? "Draft changed elsewhere",
                  remote,
                });}
            } else {
              publish({
                phase: response.status >= 500 ? "local" : "error",
                error: detail.error ?? "Draft could not sync",
              });
            }
            break;
          }
          const document = decodeDraft(result);
          if (document.id !== id) {
            throw new Error("Draft response changed identity");
          }
          replica.applyPatch(
            snapshotPatch(document.revision, document, [mutation.id]),
          );
          await replica.flush();
          deliverable.delete(mutation.id);
          publish({
            phase: replica.pending().length ? "local" : "saved",
            error: null,
          });
          // Metadata is small; it is never the source of authored content.
          void options.cache.save(library.entries).catch(() => undefined);
        }
      } catch (error) {
        publish({
          phase: "error",
          error: error instanceof Error
            ? error.message
            : "Draft persistence failed",
        });
      } finally {
        draining = false;
      }
    }
    async function refresh(): Promise<void> {
      if (fetching) return fetching;
      fetching = (async () => {
        await hydrate();
        try {
          const response = await options.request(
            `/api/drafts/${encodeURIComponent(id)}`,
          );
          if (response.status === 404 && replica.get()) return;
          if (!response.ok) {
            throw new Error(
              response.status === 404
                ? "Draft not found"
                : "Could not refresh draft",
            );
          }
          const document = decodeDraft(await response.json());
          assertActive();
          if (document.id !== id) {
            throw new Error("Draft response changed identity");
          }
          replica.applyPatch(snapshotPatch(document.revision, document, []));
          publish({
            phase: snapshot.phase === "conflict"
              ? "conflict"
              : replica.pending().length
              ? "local"
              : "saved",
            error: snapshot.phase === "conflict" ? snapshot.error : null,
          });
        } catch (error) {
          if (!replica.get()) {
            publish({
              phase: "error",
              error: error instanceof Error
                ? error.message
                : "Draft unavailable",
            });
          }
        }
      })().finally(() => {
        fetching = undefined;
      });
      return fetching;
    }
    async function change(
      change: DraftChange,
      expected?: number,
    ): Promise<void> {
      await hydrate();
      assertActive();
      if (snapshot.phase === "conflict" || snapshot.phase === "error") {
        throw new Error(snapshot.error ?? "Resolve this draft before saving");
      }
      if (change.type !== "create" && !replica.get()) await refresh();
      const args: DraftMutationArgs = {
        document_id: id,
        expected_revision: expected ?? expectedRevision(replica.get(), change),
        change,
        authored_at_ms: Date.now(),
      };
      try {
        await replica.mutateDurably("change", args, crypto.randomUUID());
        assertActive();
      } catch (error) {
        publish({
          phase: "error",
          error:
            "Could not save on this device. Keep this editor open and export your text.",
        });
        throw error;
      }
    }
    return {
      get: () => snapshot,
      subscribe: (listener: () => void) => {
        subscribers.add(listener);
        return () => {
          subscribers.delete(listener);
        };
      },
      hydrate,
      refresh,
      change,
      whenSynced: (): Promise<void> =>
        new Promise((resolve, reject) => {
          const finish = (error?: Error): void => {
            clearTimeout(timer);
            subscribers.delete(check);
            options.signal.removeEventListener("abort", abort);
            if (error) reject(error);
            else resolve();
          };
          const check = (): void => {
            if (
              snapshot.phase === "saved" && replica.pending().length === 0
            ) finish();
            else if (
              snapshot.phase === "error" || snapshot.phase === "conflict"
            ) finish(new Error(snapshot.error ?? "Draft could not sync"));
          };
          const abort = (): void =>
            finish(new Error("Draft account was closed"));
          const timer = setTimeout(() =>
            finish(
              new Error(
                "Saved on this device; still waiting for the server. The source has been kept.",
              ),
            ), 12000);
          subscribers.add(check);
          options.signal.addEventListener("abort", abort, { once: true });
          check();
        }),
      retry: async () => {
        await hydrate();
        if (snapshot.phase !== "conflict") {
          publish({
            phase: replica.pending().length ? "local" : "saved",
            error: null,
          });
          replica.resend();
        }
      },
      /** Only after an explicit recovery decision; durable confirmation keeps
       * a reload from resending a branch the user chose to abandon. */
      useRemote: async () => {
        await hydrate();
        if (draining) throw new Error("Wait for the current save to finish");
        await replica.confirmDurably(replica.pending().map((m) => m.id));
        deliverable.clear();
        const remote = snapshot.remote;
        replica.applyPatch(snapshotPatch(remote?.revision ?? 0, remote, []), {
          force: true,
        });
        await replica.flush();
        publish({ phase: "saved", remote: null, error: null });
        await refresh();
      },
      dispose: async () => {
        sealed = true;
        subscribers.clear();
        await replica.dispose();
      },
    };
  }
  function document(id: string) {
    assertActive();
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(id)) {
      throw new Error("Invalid document id");
    }
    let owner = owners.get(id);
    if (!owner) {
      owner = makeOwner(id);
      owners.set(id, owner);
    }
    return owner;
  }
  async function refresh(): Promise<void> {
    if (refreshing) return refreshing;
    refreshing = (async () => {
      try {
        const response = await options.request("/api/drafts");
        if (!response.ok) {
          throw new Error(
            response.status === 404
              ? "Drafts require the updated Cowboy server"
              : "Could not refresh drafts",
          );
        }
        const payload = await response.json() as { entries: unknown[] };
        if (!Array.isArray(payload.entries)) {
          throw new Error("Invalid draft library");
        }
        const entries = payload.entries.map((e) =>
          draftMetadata(
            decodeDraft({ ...(e as object), body: "", attachments: [] }),
          )
        );
        assertActive();
        library = { entries, loaded: true, error: null };
        for (const owner of owners.values()) updateIndex(owner.get().document);
        await options.cache.save(library.entries);
        emit();
        for (const owner of owners.values()) void owner.retry();
      } catch (error) {
        library = {
          ...library,
          loaded: true,
          error: error instanceof Error ? error.message : "Drafts unavailable",
        };
        emit();
      }
    })().finally(() => {
      refreshing = undefined;
    });
    return refreshing;
  }
  async function start(): Promise<void> {
    return starting ??= (async () => {
      try {
        const cached = await options.cache.load();
        assertActive();
        library = { entries: cached ?? [], loaded: true, error: null };
        emit();
        const ids = await options.localIds();
        await Promise.all(ids.map((id) => document(id).hydrate()));
      } catch (error) {
        library = {
          ...library,
          loaded: true,
          error: error instanceof Error
            ? error.message
            : "Local drafts unavailable",
        };
        emit();
      }
      if (!stopped) void refresh();
    })();
  }
  async function create(
    title = "Untitled",
    parent_id: string | null = null,
    kind: DraftDocument["kind"] = "document",
    body = "",
    attachments: DraftDocument["attachments"] = [],
  ): Promise<string> {
    const id = crypto.randomUUID();
    await document(id).change({
      type: "create",
      title,
      parent_id,
      kind,
      body,
      attachments,
    });
    return id;
  }
  async function dispose(): Promise<void> {
    stopped = true;
    listeners.clear();
    await Promise.all([...owners.values()].map((owner) => owner.dispose()));
  }
  options.signal.addEventListener("abort", () => {
    void dispose().catch(() => undefined);
  }, { once: true });
  return {
    document,
    create,
    start,
    refresh,
    dispose,
    get: () => library,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}
