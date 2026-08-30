// Client engine: optimistic local apply + rebase-on-patch (the Replicache model,
// scoped to one synced value). The view a caller renders is always
//   base (last confirmed authoritative state)  +  replay(pending mutations)
// so a local mutation shows instantly, and every arbiter patch re-derives the
// view from the new authoritative base with the still-unconfirmed mutations
// replayed on top — converging on the arbiter's order with no lost update and
// no ghost (a confirmed mutation is dropped from pending exactly once).

import { hashValue } from "./hash.ts";
import { applyMutation, type ArgsOf, type Mutators } from "./mutators.ts";
import type {
  ClientId,
  ClientSnapshot,
  LocalPersistence,
  Mutation,
  MutationId,
  Patch,
  SyncState,
  Version,
} from "./types.ts";

/** Recursively freeze a value so a mutator that MUTATES its input (the cardinal
 *  sin — it breaks replay convergence) throws in dev instead of silently
 *  corrupting state. O(value); gate behind `freezeForDev`, off in prod. */
function deepFreeze<V>(value: V): V {
  if (value === null || typeof value !== "object" || Object.isFrozen(value)) {
    return value;
  }
  Object.freeze(value);
  for (const k of Object.keys(value as Record<string, unknown>)) {
    deepFreeze((value as Record<string, unknown>)[k]);
  }
  return value;
}

export interface Client<T, M extends Mutators<T>> {
  /** The value to render: confirmed base + replayed pending. */
  view(): T;
  /** The confirmed (arbiter) version this client has applied up to. */
  version(): Version;
  /** Outstanding optimistic mutations not yet confirmed by the arbiter. */
  pending(): readonly Mutation[];
  /** Apply a mutator locally (instant) and return the Mutation to send to the
   *  arbiter. Re-send the SAME object on retry — its id makes the arbiter
   *  idempotent. Pass an explicit `id` when the mutation id must equal an
   *  externally-meaningful key — e.g. an optimistic row's `cmid`, so the same
   *  cmid landing in this state's value confirms (drops) exactly this pending
   *  row with no duplicate. Defaults to the client's `newId`. */
  mutate<K extends keyof M & string>(name: K, args: ArgsOf<T, M, K>, id?: MutationId): Mutation<ArgsOf<T, M, K>>;
  /** Apply locally, durably persist the pending mutation, and only then return it
   *  to the transport owner for sending. If persistence fails, the optimistic
   *  mutation is rolled back and the promise rejects. Use this for user-authored
   *  data whose source editor is cleared after this promise resolves. */
  mutateDurably<K extends keyof M & string>(
    name: K,
    args: ArgsOf<T, M, K>,
    id?: MutationId,
  ): Promise<Mutation<ArgsOf<T, M, K>>>;
  /** Drop pending mutations confirmed OUT-OF-BAND — i.e. acknowledged by a signal
   *  OTHER than this state's patch (e.g. an optimistic "submit" whose row was
   *  confirmed by a separate event stream, not by the value landing in this
   *  state). Same monotonic-fact semantics as a patch's `confirmed`, but with no
   *  base/version change. A no-op for ids not pending. */
  confirm(ids: readonly MutationId[]): void;
  /** Durably remove pending mutations before reporting a destructive local
   *  acknowledgement (for example, Discard). A failed persistence write restores
   *  the pending mutations in memory so a reload cannot silently resend content
   *  that the UI already claimed to remove. */
  confirmDurably(ids: readonly MutationId[]): Promise<void>;
  /** Move a pending mutation to the TAIL of the pending queue, so it replays
   *  last and renders at the very end. The retry-to-end gesture (WeChat-style):
   *  clicking retry re-anchors that row after everything, and N retries land in
   *  click order. Same id ⇒ still idempotent (no duplicate). Local-only: it
   *  reorders the optimistic view, never the converged base. No-op if `id` isn't
   *  pending or is already last. */
  bump(id: MutationId): void;
  /** Fold an arbiter patch into the base, drop confirmed pending, rebase the
   *  rest. Stale/duplicate patches are a no-op.
   *
   *  `opts.force` adopts the patch's value as the new base REGARDLESS of version
   *  — for a reconnect RESYNC, where the arbiter's current snapshot is ground
   *  truth even if its version is lower than what this client cached (e.g. the
   *  service restarted and its version clock reset). Pending is preserved +
   *  replayed on the forced base. Without `force`, only a strictly-newer version
   *  advances the base (live-stream ordering). */
  applyPatch(patch: Patch<T>, opts?: { force?: boolean }): void;
  /** Restore base + pending from the configured `LocalPersistence` (instant first
   *  paint on reload + a durable outbox for unconfirmed optimistic mutations).
   *  Call once before connecting; a no-op if no persistence is configured or
   *  nothing is stored. The restored pending should then be re-sent (see the
   *  reconnect resend) — its ids keep the arbiter idempotent. */
  hydrate(): Promise<void>;
  /** Persist the current snapshot NOW, bypassing the save debounce — call on
   *  `pagehide`/`beforeunload` so an in-flight change isn't lost. No-op without
   *  persistence. */
  flush(): Promise<void>;
}

export interface ClientOpts<T, M extends Mutators<T>> {
  clientId: ClientId;
  mutators: M;
  /** The arbiter's current state at connect (version 0 + initial value, or a
   *  resync snapshot). */
  initial: SyncState<T>;
  onChange?: (view: T) => void;
  /** Mutation-id factory (default `clientId:counter`). Inject for deterministic
   *  tests. Must be globally unique across clients. */
  newId?: () => MutationId;
  /** Called when an applied patch's `valueHash` disagrees with this client's own
   *  value at that version (no pending) — i.e. a divergence/integrity failure
   *  (corrupt wire round-trip, non-deterministic mutator, …). Default:
   *  `console.error`. Throw here to fail loud in tests. */
  onDiverge?: (detail: { version: Version; expected: string; got: string }) => void;
  /** Dev-only: deep-freeze the confirmed base so a mutator that mutates its
   *  input throws. O(value) per patch; leave off in production. */
  freezeForDev?: boolean;
  /** App-side persistence backend (e.g. state/sync-idb). When set, the
   *  client debounce-saves its `ClientSnapshot<T>` ({base, pending} = a durable
   *  outbox) on every change and can `hydrate()` from it. Omit to keep the client
   *  purely in-memory (the default). */
  local?: LocalPersistence<ClientSnapshot<T>>;
  /** Debounce window (ms) for persistence saves. Default 250. */
  saveDebounceMs?: number;
}

export function createClient<T, M extends Mutators<T>>(opts: ClientOpts<T, M>): Client<T, M> {
  const { clientId, mutators, onChange, freezeForDev, local } = opts;
  const saveDebounceMs = opts.saveDebounceMs ?? 250;
  const onDiverge = opts.onDiverge ??
    ((d: { version: Version; expected: string; got: string }): void => {
      console.error(`sync: divergence at v${String(d.version)}: expected ${d.expected}, got ${d.got}`);
    });
  let seq = 0;
  const newId = opts.newId ?? ((): MutationId => `${clientId}:${String(++seq)}`);

  const freeze = (s: SyncState<T>): SyncState<T> =>
    freezeForDev ? { version: s.version, value: deepFreeze(s.value) } : s;

  let base: SyncState<T> = freeze(opts.initial);
  let queue: Mutation[] = [];
  let viewValue: T = base.value;
  // `hydrate()` normally completes before a transport connects. Some browsers,
  // however, can leave an IndexedDB open/read pending long enough that the app
  // deliberately opens its socket first. Track live changes made during that
  // read so the late cache result can merge its durable outbox without
  // overwriting a newer server base. Confirmations are tracked separately: a
  // patch can acknowledge an id before that persisted mutation has been loaded.
  let stateRevision = 0;
  let baseRevision = 0;
  let hasAppliedPatch = false;
  let hydrationComplete = local === undefined;
  // Mutation ids are globally unique. Remember confirmations for this client
  // lifetime so a cache read that STARTS after the corresponding socket patch
  // cannot resurrect an already-accepted outbox row.
  const confirmedFacts = new Set<MutationId>();
  const noteHydrateConfirmations = (ids: readonly MutationId[]): void => {
    if (ids.length === 0) return;
    for (const id of ids) confirmedFacts.add(id);
  };

  // Debounced app-side persistence of {base, pending}. No-op without `local`.
  let saveTimer: ReturnType<typeof setTimeout> | undefined = undefined;
  // IndexedDB transactions are asynchronous and separate save calls may finish
  // out of order. Serialize them so an older base/pending snapshot can never
  // overwrite a newer acknowledgement snapshot.
  let saveTail: Promise<void> = Promise.resolve();
  const snapshot = (): ClientSnapshot<T> => ({ base, pending: [...queue] });
  const persist = (next: ClientSnapshot<T>): Promise<void> => {
    if (local === undefined) {
      return Promise.resolve();
    }
    const write = saveTail.then(() => local.save(next));
    // Keep the serialization chain usable after an explicit strict write fails;
    // callers still receive the original rejection through `write`.
    saveTail = write.catch(() => undefined);
    return write;
  };
  const scheduleSave = (): void => {
    if (local === undefined) {
      return;
    }
    if (saveTimer !== undefined) {
      clearTimeout(saveTimer);
    }
    saveTimer = setTimeout(() => {
      saveTimer = undefined;
      // Background cache persistence remains best-effort. Critical callers use
      // `mutateDurably`/`flush`, which observe and surface a strict backend error.
      void persist(snapshot()).catch(() => undefined);
    }, saveDebounceMs);
  };

  const recompute = (): void => {
    let v = base.value;
    for (const m of queue) {
      v = applyMutation(mutators, v, m);
    }
    viewValue = v;
  };

  return {
    view: (): T => viewValue,
    version: (): Version => base.version,
    pending: (): readonly Mutation[] => queue,

    mutate<K extends keyof M & string>(name: K, args: ArgsOf<T, M, K>, id?: MutationId): Mutation<ArgsOf<T, M, K>> {
      const m: Mutation<ArgsOf<T, M, K>> = { id: id ?? newId(), client: clientId, name, args };
      queue.push(m);
      // Incremental: apply on the current view (== replaying just this one on top
      // of the already-replayed queue), equivalent to a full recompute.
      viewValue = applyMutation(mutators, viewValue, m);
      stateRevision += 1;
      onChange?.(viewValue);
      scheduleSave();
      return m;
    },

    async mutateDurably<K extends keyof M & string>(
      name: K,
      args: ArgsOf<T, M, K>,
      id?: MutationId,
    ): Promise<Mutation<ArgsOf<T, M, K>>> {
      const m = this.mutate(name, args, id);
      try {
        await this.flush();
      } catch (error) {
        // Nothing has been handed to the transport yet. Remove the optimistic
        // row so the still-mounted editor remains the single recovery source.
        this.confirm([m.id]);
        throw error;
      }
      return m;
    },

    confirm(ids: readonly MutationId[]): void {
      noteHydrateConfirmations(ids);
      if (ids.length === 0) {
        return;
      }
      const set = new Set<MutationId>(ids);
      const next = queue.filter((m) => !set.has(m.id));
      if (next.length === queue.length) {
        return; // none pending — no-op
      }
      queue = next;
      recompute();
      stateRevision += 1;
      onChange?.(viewValue);
      scheduleSave();
    },

    async confirmDurably(ids: readonly MutationId[]): Promise<void> {
      if (ids.length === 0) {
        return;
      }
      const previous = queue;
      const set = new Set<MutationId>(ids);
      const next = queue.filter((m) => !set.has(m.id));
      if (next.length === queue.length) {
        return;
      }
      queue = next;
      recompute();
      stateRevision += 1;
      onChange?.(viewValue);
      try {
        await this.flush();
      } catch (error) {
        // A transport patch or another optimistic mutation may have landed while
        // the storage transaction was pending. Restore only the mutations this
        // durable confirmation removed; preserve every concurrent queue change.
        const previousIds = new Set(previous.map((m) => m.id));
        const currentById = new Map(queue.map((m) => [m.id, m]));
        const restored: Mutation[] = [];
        for (const mutation of previous) {
          const current = currentById.get(mutation.id);
          if (set.has(mutation.id)) {
            restored.push(current ?? mutation);
          } else if (current !== undefined) {
            restored.push(current);
          }
        }
        restored.push(...queue.filter((m) => !previousIds.has(m.id)));
        queue = restored;
        recompute();
        stateRevision += 1;
        onChange?.(viewValue);
        scheduleSave();
        throw error;
      }
    },

    bump(id: MutationId): void {
      const i = queue.findIndex((m) => m.id === id);
      if (i === -1 || i === queue.length - 1) {
        return; // not pending or already last
      }
      const m = queue[i];
      if (m === undefined) {
        return;
      }
      queue = [...queue.slice(0, i), ...queue.slice(i + 1), m];
      recompute();
      stateRevision += 1;
      onChange?.(viewValue);
      scheduleSave();
    },

    applyPatch(patch: Patch<T>, applyOpts?: { force?: boolean }): void {
      noteHydrateConfirmations(patch.confirmed);
      let changed = false;
      // CONFIRMATIONS ARE MONOTONIC FACTS — process them from EVERY patch, even a
      // reordered/older/dup one. A patch's value may be stale (an absolute
      // snapshot we already advanced past), but its `confirmed` still tells us a
      // pending mutation is now folded into the truth. Skipping it would leave
      // that mutation in `queue` and replay it on top of a newer base that
      // already includes it → double-count → divergence (caught by the fuzz).
      if (patch.confirmed.length > 0) {
        const confirmed = new Set<MutationId>(patch.confirmed);
        const next = queue.filter((m) => !confirmed.has(m.id));
        if (next.length !== queue.length) {
          queue = next;
          changed = true;
        }
      }
      // VALUE: only a strictly-newer absolute snapshot advances the base; an
      // older/duplicate snapshot's value is ignored (idempotent). (An op-patch
      // would additionally require fromVersion === base.version + gap → resync.)
      if (applyOpts?.force === true || patch.toVersion > base.version) {
        base = freeze({ version: patch.toVersion, value: patch.apply(base.value) });
        baseRevision += 1;
        hasAppliedPatch = true;
        changed = true;
      }
      // Machine-checked convergence: once we're AT this patch's version with NO
      // pending, our base IS the authoritative value, so its hash must match the
      // patch's. A mismatch means a corrupt round-trip or a non-deterministic
      // mutator — surface it loudly rather than silently diverge.
      if (
        patch.valueHash !== undefined &&
        queue.length === 0 &&
        base.version === patch.toVersion
      ) {
        const got = hashValue(base.value);
        if (got !== patch.valueHash) {
          onDiverge({ version: patch.toVersion, expected: patch.valueHash, got });
        }
      }
      if (changed) {
        recompute();
        stateRevision += 1;
        onChange?.(viewValue);
        scheduleSave();
      }
    },

    async hydrate(): Promise<void> {
      if (local === undefined || hydrationComplete) {
        return;
      }
      try {
        const startedStateRevision = stateRevision;
        const startedBaseRevision = baseRevision;
        const snap = await local.load();
        if (snap === null) {
          return;
        }

        // Any live patch is newer than the browser cache, including one received
        // while IndexedDB was still enumerating keys before this hydrate call
        // started. Keep it. If only local mutations happened, the cached base is
        // still useful and those mutations are replayed on top below.
        if (!hasAppliedPatch && baseRevision === startedBaseRevision) {
          base = freeze(snap.base);
        }

        // Durable mutations predate anything authored after this hydrate began,
        // so restore them first, then append current-only mutations. For an id
        // present in both places the live copy wins. Never resurrect an id that
        // a socket patch/user echo has already confirmed on this page.
        const currentById = new Map(queue.map((mutation) => [mutation.id, mutation]));
        const persistedIds = new Set<MutationId>();
        const merged: Mutation[] = [];
        let skippedConfirmed = false;
        for (const mutation of snap.pending) {
          persistedIds.add(mutation.id);
          if (confirmedFacts.has(mutation.id)) {
            skippedConfirmed = true;
            continue;
          }
          merged.push(currentById.get(mutation.id) ?? mutation);
        }
        merged.push(
          ...queue.filter((mutation) =>
            !persistedIds.has(mutation.id) &&
            !confirmedFacts.has(mutation.id)
          ),
        );
        queue = merged;
        recompute();
        stateRevision += 1;
        onChange?.(viewValue);

        // A concurrent patch/mutation may already have persisted a snapshot that
        // did not yet contain the restored outbox. Likewise, a stale record may
        // still contain an id the server confirmed. Serialize one corrected write
        // before reporting hydration complete.
        if (
          stateRevision !== startedStateRevision + 1 ||
          skippedConfirmed ||
          baseRevision !== startedBaseRevision
        ) {
          await persist(snapshot());
        }
      } finally {
        // Hydration is a one-shot startup operation. Stop retaining confirmation
        // ids afterward so a long-running client does not grow this set forever,
        // and prevent a later accidental cache read from replacing live state.
        hydrationComplete = true;
        confirmedFacts.clear();
      }
    },

    async flush(): Promise<void> {
      if (local === undefined) {
        return;
      }
      if (saveTimer !== undefined) {
        clearTimeout(saveTimer);
        saveTimer = undefined;
      }
      await persist(snapshot());
    },
  };
}
