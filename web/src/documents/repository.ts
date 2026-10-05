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
  type DraftContent,
  draftContent,
  type DraftDocument,
  type DraftMetadata,
  draftMetadata,
  type DraftMutationArgs,
  draftMutators,
  expectedRevision,
  mergeDraftContent,
  sameDraftContent,
} from "./model";

/** Automatic rebases per drain; a server that keeps refusing is a bug, not
 * a race, and falls back to the recoverable conflict state. */
const MAX_REBASES = 8;

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
/** The local edit and the newer text are too divergent to merge. */
export class DraftMergeError extends Error {
  constructor() {
    super("This draft changed too much elsewhere to merge automatically");
  }
}

interface RepositoryOptions {
  persistence(
    id: string,
  ): LocalPersistence<ClientSnapshot<DraftDocument | null>>;
  cache: ProductCache<readonly DraftMetadata[]>;
  localIds(): Promise<string[]>;
  request(path: string, init?: RequestInit): Promise<Response>;
  signal: AbortSignal;
  /** One-line user notice, e.g. a version kept as a separate draft. */
  notify?(message: string): void;
}

/** Per-document durable replicas keep large bodies out of the library index.
 * Outbox delta merges preserve other tabs' authored operations. The server
 * arbitrates versions. A refused write is merged three-way with the newer
 * server text (Obsidian Sync style) and resent; metadata is last-writer-wins.
 * Only an unmergeable branch stops in the recoverable conflict state. */
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
    /** Content the server held at `bodyRevision`, for a pending write queued
     * before writes recorded their ancestor. */
    async function historicAncestor(
      bodyRevision: number,
    ): Promise<DraftContent | null> {
      try {
        const response = await options.request(
          `/api/drafts/${encodeURIComponent(id)}/history`,
        );
        if (!response.ok) return null;
        const history = await response.json() as unknown[];
        for (const entry of history) {
          const past = decodeDraft(entry);
          if (past.id === id && past.body_revision === bodyRevision) {
            return draftContent(past);
          }
        }
      } catch {
        // No ancestor: the local branch is preserved as a copy instead.
      }
      return null;
    }
    /** Keep authored text that cannot be merged as its own draft. */
    async function preserve(ours: DraftContent): Promise<void> {
      if (!ours.body.trim() && ours.attachments.length === 0) return;
      const view = replica.get();
      await create(
        `${view?.title ?? "Untitled"} (conflicted copy)`,
        view?.parent_id ?? null,
        "document",
        ours.body,
        ours.attachments,
      );
      options.notify?.(
        "This draft was edited on two devices. Your version was kept as a copy.",
      );
    }
    /** Resolve a refused mutation against `remote` without ever blocking the
     * document. Writes fold every pending write into a single merged write;
     * metadata changes reapply the local intent. Authored text that cannot be
     * merged (no ancestor, too divergent, deleted elsewhere) is kept as a
     * separate draft and this document adopts the server state. */
    async function rebase(
      mutation: Mutation,
      remote: DraftDocument | null,
    ): Promise<boolean> {
      const args = mutation.args as DraftMutationArgs;
      const { change } = args;
      const view = replica.get();
      const writes = replica.pending()
        .filter((m) => {
          const type = (m.args as DraftMutationArgs).change.type;
          return type === "write" || type === "create";
        })
        .map((m) => m.id);
      let superseded = [mutation.id];
      let replacement:
        | { change: DraftChange; expected: number; base?: DraftContent }
        | null = null;
      switch (change.type) {
        case "create":
        case "write": {
          superseded = writes;
          const ours = view ? draftContent(view) : draftContent(
            change.type === "create" ? change : { body: "", attachments: [] },
          );
          const theirs = remote && !remote.deleted
            ? draftContent(remote)
            : null;
          if (theirs && sameDraftContent(ours, theirs)) break;
          const ancestor = change.type === "create"
            ? null
            : args.base ?? await historicAncestor(args.expected_revision);
          const merged = theirs && ancestor
            ? mergeDraftContent(ancestor, ours, theirs)
            : null;
          if (!merged || !theirs || !remote) {
            await preserve(ours);
          } else if (!sameDraftContent(merged, theirs)) {
            replacement = {
              change: { type: "write", ...merged },
              expected: remote.body_revision,
              base: theirs,
            };
          }
          break;
        }
        case "rename":
          if (remote && !remote.deleted && remote.title !== change.title) {
            replacement = { change, expected: remote.metadata_revision };
          }
          break;
        case "move":
          if (
            remote && !remote.deleted && remote.parent_id !== change.parent_id
          ) {
            replacement = { change, expected: remote.metadata_revision };
          }
          break;
        case "trash":
        case "restore":
          if (remote && remote.deleted !== (change.type === "trash")) {
            replacement = { change, expected: remote.revision };
          }
          break;
      }
      await replica.confirmDurably(superseded);
      for (const id of superseded) deliverable.delete(id);
      replica.applyPatch(snapshotPatch(remote?.revision ?? 0, remote, []), {
        force: true,
      });
      if (replacement) {
        await replica.mutateDurably("change", {
          document_id: id,
          expected_revision: replacement.expected,
          change: replacement.change,
          authored_at_ms: Date.now(),
          ...(replacement.base ? { base: replacement.base } : {}),
        }, crypto.randomUUID());
      } else {
        await replica.flush();
      }
      return true;
    }
    async function drain(): Promise<void> {
      if (draining || stopped || sealed || snapshot.phase === "conflict") {
        return;
      }
      draining = true;
      let rebases = 0;
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
              if (rebases < MAX_REBASES && await rebase(mutation, remote)) {
                rebases++;
                publish({
                  phase: replica.pending().length ? "local" : "saved",
                  error: null,
                  remote: null,
                });
                continue;
              }
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
    /** `base` is the content a write was authored against. It identifies the
     * edit by content rather than by a revision number the caller may hold
     * stale, and is merged with anything that arrived since. */
    async function change(
      change: DraftChange,
      expected?: number,
      base?: DraftContent,
    ): Promise<void> {
      await hydrate();
      assertActive();
      // A delivery problem never refuses local durability: the authored change
      // joins the outbox so the editor, navigation and menus stay usable.
      if (change.type !== "create" && !replica.get()) await refresh();
      const view = replica.get();
      let ancestor: DraftContent | undefined;
      if (change.type === "write" && view && (base || expected === undefined)) {
        ancestor = draftContent(view);
        if (base && !sameDraftContent(base, ancestor)) {
          const merged = mergeDraftContent(base, change, ancestor);
          if (!merged) {
            throw new DraftMergeError();
          }
          change = { type: "write", ...merged };
        }
        expected = view.body_revision;
      }
      const args: DraftMutationArgs = {
        document_id: id,
        expected_revision: expected ?? expectedRevision(view, change),
        change,
        authored_at_ms: Date.now(),
        ...(ancestor ? { base: ancestor } : {}),
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
      /** Server revision announced by the push channel. */
      revisionKnown: (revision: number): boolean =>
        (replica.baseValue()?.revision ?? 0) >= revision,
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
  /** Pushed `{document id: metadata}` of this principal's documents changed
   * since the controller started. The library index adopts newer metadata
   * directly; an open document fetches only a revision it has not seen. */
  function announce(value: unknown): void {
    if (stopped || value === null || typeof value !== "object") return;
    for (const entry of Object.values(value)) {
      let metadata: DraftDocument;
      try {
        metadata = decodeDraft({
          ...(entry as object),
          body: "",
          attachments: [],
        });
      } catch {
        continue;
      }
      if (library.loaded) updateIndex(metadata);
      const owner = owners.get(metadata.id);
      if (owner && !owner.revisionKnown(metadata.revision)) {
        void owner.refresh();
      }
    }
  }
  options.signal.addEventListener("abort", () => {
    void dispose().catch(() => undefined);
  }, { once: true });
  return {
    document,
    create,
    start,
    refresh,
    announce,
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
