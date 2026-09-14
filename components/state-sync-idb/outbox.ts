import type {
  ClientSnapshot,
  LocalPersistence,
  Mutation,
} from "@cowboy/state-sync";
import { IdbPersistenceError } from "./errors.ts";

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" &&
    Object.getPrototypeOf(value) === Object.prototype;
}

function fields(
  value: Record<string, unknown>,
  keys: readonly string[],
): boolean {
  return Object.keys(value).length === keys.length &&
    keys.every((key) => Object.hasOwn(value, key));
}

// Mutation arguments are JSON-plain by the sync contract. Bound traversal per
// snapshot, including corrupt/cyclic clones. Do not stringify image payloads
// into additional full-size identity strings on the browser's main thread.
function argumentValidator(): (value: unknown) => void {
  let remaining = 100_000;
  const visit = (item: unknown, depth: number): void => {
    if (--remaining < 0 || depth > 64) {
      throw new IdbPersistenceError("snapshot_invalid");
    }
    if (
      item === null || typeof item === "string" || typeof item === "boolean"
    ) {
      return;
    }
    if (typeof item === "number" && Number.isFinite(item)) {
      return;
    }
    if (Array.isArray(item)) {
      for (const entry of item) visit(entry, depth + 1);
      return;
    }
    if (record(item)) {
      for (const entry of Object.values(item)) visit(entry, depth + 1);
      return;
    }
    throw new IdbPersistenceError("snapshot_invalid");
  };
  return (value) => visit(value, 0);
}

/** Exact equality of previously validated JSON, independent of object key order.
 * Never substitute the engine's 32-bit convergence hash for identity equality.
 */
function sameArguments(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (Array.isArray(left)) {
    return Array.isArray(right) && left.length === right.length &&
      left.every((value, index) => sameArguments(value, right[index]));
  }
  if (!record(left) || !record(right)) return false;
  const keys = Object.keys(left);
  return keys.length === Object.keys(right).length &&
    keys.every((key) =>
      Object.hasOwn(right, key) && sameArguments(left[key], right[key])
    );
}

function sameMutation(left: Mutation, right: Mutation): boolean {
  return left.client === right.client && left.name === right.name &&
    sameArguments(left.args, right.args);
}

interface Decoded<T> {
  snapshot: ClientSnapshot<T>;
  identities: Map<string, Mutation>;
}

function decode<T>(value: unknown): Decoded<T> {
  if (
    !record(value) || !fields(value, ["base", "pending"]) ||
    !record(value.base) || !fields(value.base, ["version", "value"]) ||
    typeof value.base.version !== "number" ||
    !Number.isSafeInteger(value.base.version) || value.base.version < 0 ||
    !Array.isArray(value.pending) || value.pending.length > 4096
  ) throw new IdbPersistenceError("snapshot_invalid");
  const identities = new Map<string, Mutation>();
  const validateArguments = argumentValidator();
  for (const mutation of value.pending) {
    if (
      !record(mutation) ||
      !fields(mutation, ["id", "client", "name", "args"]) ||
      ![mutation.id, mutation.client, mutation.name].every((field) =>
        typeof field === "string" && field.length > 0 && field.length <= 4096
      ) || identities.has(mutation.id as string)
    ) throw new IdbPersistenceError("snapshot_invalid");
    validateArguments(mutation.args);
    identities.set(mutation.id as string, mutation as unknown as Mutation);
  }
  // This validates the envelope/mutation identity, not the application's T or
  // mutator registry. It is never an authorization or a general dataset codec.
  return { snapshot: value as unknown as ClientSnapshot<T>, identities };
}

function cloned<T>(value: T): T {
  try {
    return structuredClone(value);
  } catch {
    throw new IdbPersistenceError("snapshot_invalid");
  }
}

/** Pure delta against the last snapshot actually observed/saved by THIS client.
 * The caller must execute read + merge + put in ONE readwrite transaction.
 */
export function mergeOutbox<T>(
  previous: ClientSnapshot<T> | null,
  nextValue: ClientSnapshot<T>,
  stored: unknown,
): ClientSnapshot<T> {
  const next = decode<T>(nextValue);
  const before = previous === null ? undefined : decode<T>(previous);
  const current = stored === undefined ? undefined : decode<T>(stored);
  for (const source of [before, current]) {
    if (!source) continue;
    for (const [id, identity] of source.identities) {
      for (const other of [before, current, next]) {
        const observed = other?.identities.get(id);
        if (observed !== undefined && !sameMutation(observed, identity)) {
          throw new IdbPersistenceError("outbox_conflict");
        }
      }
    }
  }
  const retained = (current?.snapshot.pending ?? []).filter((mutation) =>
    !before?.identities.has(mutation.id) || next.identities.has(mutation.id)
  );
  // A peer's removal of an already-observed id wins over our stale snapshot.
  // Only a genuinely new local id can be added to the shared outbox.
  const own = next.snapshot.pending.filter((mutation) =>
    !before?.identities.has(mutation.id) || current?.identities.has(mutation.id)
  );
  const pending: Mutation[] = [];
  let index = 0;
  for (const mutation of retained) {
    if (next.identities.has(mutation.id)) pending.push(own[index++]!);
    else pending.push(mutation);
  }
  pending.push(...own.slice(index));
  if (pending.length > 4096) throw new IdbPersistenceError("snapshot_invalid");

  // Base is only an offline paint cache, never transport authority. Preserve a
  // newer cached version, except when this client observed an explicit version
  // reset since its own previous snapshot. Live forced resync remains decisive.
  const reset = before !== undefined &&
    next.snapshot.base.version < before.snapshot.base.version;
  const base = !reset && current !== undefined &&
      current.snapshot.base.version > next.snapshot.base.version
    ? current.snapshot.base
    : next.snapshot.base;
  return { base, pending };
}

export interface OutboxAccess {
  assertActive(): void;
  own<R>(task: () => Promise<R>): Promise<R>;
  load(): Promise<unknown>;
  update(merge: (stored: unknown) => unknown): Promise<void>;
}

/** One borrowed replicated client per record handle. No disposal authority. */
export function createOutboxPersistence<T>(
  access: OutboxAccess,
): LocalPersistence<ClientSnapshot<T>> {
  let previous: ClientSnapshot<T> | null = null;
  let tail = Promise.resolve();
  let hydration: Promise<ClientSnapshot<T> | null> | undefined;
  let loading = false;
  let loaded: ClientSnapshot<T> | null = null;
  let adopted = false;
  let issued: ClientSnapshot<T> | null | undefined;
  let fence: IdbPersistenceError | undefined;
  const serial = <R>(task: () => Promise<R>): Promise<R> => {
    const result = tail.then(() => {
      if (fence) throw fence;
      return task();
    });
    tail = result.then(() => undefined, () => undefined);
    return result;
  };
  return {
    load: (): Promise<ClientSnapshot<T> | null> =>
      access.own(() => {
        if (hydration) return hydration;
        loading = true;
        hydration = serial(async () => {
          try {
            const value = await access.load();
            loaded = value === undefined ? null : decode<T>(value).snapshot;
            return loaded;
          } catch (error) {
            // Failed acquisition is not proof of an empty outbox. This owner
            // cannot replace unread obligations with its initial snapshot.
            fence = error instanceof IdbPersistenceError
              ? error
              : new IdbPersistenceError("request_failed");
            loading = false;
            throw fence;
          }
        }).then((value) => {
          issued = cloned(value);
          return issued;
        });
        return hydration;
      }),
    acceptLoadedSnapshot: (value): void => {
      access.assertActive();
      if (fence) throw fence;
      if (!loading || adopted || value !== issued) {
        throw new IdbPersistenceError("outbox_conflict");
      }
      // Never trust a caller-mutated copy as the baseline. Its object identity
      // correlates the handoff; the privately retained clone supplies the data.
      previous = loaded;
      adopted = true;
      loading = false;
    },
    save: (value): Promise<void> =>
      access.own(async () => {
        // An already-captured snapshot cannot acknowledge data whose load has
        // not reached the client yet. Reject this race instead of deleting it.
        if (loading) throw new IdbPersistenceError("outbox_loading");
        const next = cloned(value);
        decode<T>(next);
        await serial(async () => {
          try {
            await access.update((stored) =>
              mergeOutbox(previous, next, stored)
            );
            previous = next; // only after the real transaction commits
          } catch (error) {
            if (
              error instanceof IdbPersistenceError &&
              (error.code === "outbox_conflict" ||
                error.code === "snapshot_invalid")
            ) fence = error;
            throw error;
          }
        });
      }),
  };
}
