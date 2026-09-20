// The local replica: last known server-derived state, persisted per dataset so
// the app can open, read and compose before the Hub answers
// (docs/offline-first-sync.md §Architecture 1). Every value here is a paint
// cache. The Hub's next broadcast replaces it; nothing in it is an obligation
// and nothing in it is ever sent.
import type {
  ConfigOption,
  Envelope,
  MachineSummary,
  SessionMeta,
} from "./protocol.ts";
import type {
  ProductCache,
  ProductCacheScope,
} from "./productSyncDatabase.ts";

export interface ReplicaSessions {
  readonly receivedAt: number;
  readonly sessions: readonly SessionMeta[];
}

export interface ReplicaMachines {
  readonly receivedAt: number;
  readonly revision: number;
  readonly machines: readonly MachineSummary[];
}

export interface ReplicaTail {
  readonly receivedAt: number;
  readonly lastSeq: number;
  readonly reachedStart: boolean;
  readonly events: readonly Envelope[];
  readonly configOptions?: readonly ConfigOption[];
  /** The `ETag` of the bootstrap response this tail was reconciled against.
   *  Replaying it as `If-None-Match` turns reopening an unchanged session
   *  into a 304 (docs/offline-first-sync.md §Reopening a session). */
  readonly etag?: string;
}

export interface ReplicaDelivery {
  /** Mutation ids the user left unsent after a timeout. Never auto-resent. */
  readonly held: readonly string[];
}

export interface ReplicaDatabase {
  cache<T>(scope: ProductCacheScope): ProductCache<T>;
  cacheSessions(state: "tail" | "delivery"): Promise<string[]>;
  discardCaches(): Promise<void>;
}

/** Trailing quiet window before a scheduled write lands. */
export const REPLICA_WRITE_DEBOUNCE_MS = 400;
/** Upper bound on how long a continuously changing value can defer its write. */
export const REPLICA_WRITE_MAX_WAIT_MS = 4_000;

const SESSION_ID = /^[A-Za-z0-9_-]{1,128}$/;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function finite(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function decodeReplicaSessions(value: unknown): ReplicaSessions | null {
  if (!isRecord(value) || !finite(value.receivedAt) || !Array.isArray(value.sessions)) {
    return null;
  }
  const sessions: SessionMeta[] = [];
  for (const row of value.sessions) {
    if (
      !isRecord(row) || typeof row.id !== "string" || !SESSION_ID.test(row.id) ||
      typeof row.provider !== "string" || typeof row.cwd !== "string" ||
      typeof row.title !== "string" || typeof row.status !== "string"
    ) return null;
    sessions.push(row as unknown as SessionMeta);
  }
  return { receivedAt: value.receivedAt, sessions };
}

export function decodeReplicaMachines(value: unknown): ReplicaMachines | null {
  if (
    !isRecord(value) || !finite(value.receivedAt) || !finite(value.revision) ||
    !Array.isArray(value.machines)
  ) return null;
  const machines: MachineSummary[] = [];
  for (const row of value.machines) {
    if (
      !isRecord(row) || typeof row.id !== "string" || row.id.length === 0 ||
      typeof row.display_name !== "string" || !Array.isArray(row.workspaces) ||
      !Array.isArray(row.components) || !isRecord(row.capacity)
    ) return null;
    machines.push(row as unknown as MachineSummary);
  }
  return { receivedAt: value.receivedAt, revision: value.revision, machines };
}

export function decodeReplicaTail(value: unknown): ReplicaTail | null {
  if (
    !isRecord(value) || !finite(value.receivedAt) || !finite(value.lastSeq) ||
    typeof value.reachedStart !== "boolean" || !Array.isArray(value.events)
  ) return null;
  const events: Envelope[] = [];
  let previous = Number.NEGATIVE_INFINITY;
  for (const row of value.events) {
    if (
      !isRecord(row) || typeof row.session_id !== "string" ||
      !finite(row.seq) || row.seq <= previous || typeof row.kind !== "string"
    ) return null;
    previous = row.seq;
    events.push(row as unknown as Envelope);
  }
  const configOptions = Array.isArray(value.configOptions)
    ? value.configOptions.every((option) =>
        isRecord(option) && typeof option.id === "string" && Array.isArray(option.options)
      )
      ? (value.configOptions as unknown as ConfigOption[])
      : undefined
    : undefined;
  return {
    receivedAt: value.receivedAt,
    lastSeq: value.lastSeq,
    reachedStart: value.reachedStart,
    events,
    ...(configOptions !== undefined ? { configOptions } : {}),
    ...(typeof value.etag === "string" && value.etag.length > 0 &&
        value.etag.length <= 200
      ? { etag: value.etag }
      : {}),
  };
}

export function decodeReplicaDelivery(value: unknown): ReplicaDelivery | null {
  if (!isRecord(value) || !Array.isArray(value.held)) return null;
  if (!value.held.every((id) => typeof id === "string" && id.length > 0 && id.length <= 256)) {
    return null;
  }
  return { held: value.held as string[] };
}

interface ReplicaWriter<T> {
  /** Register the newest producer. The value is read when the write lands, so
   * a burst of changes costs one serialization. */
  schedule(produce: () => T | null, opts?: { readonly immediate?: boolean }): void;
  flush(): Promise<void>;
  cancel(): void;
}

function createWriter<T>(
  cache: ProductCache<T>,
  timing: { readonly debounceMs: number; readonly maxWaitMs: number },
  active: () => boolean,
  onError: (error: unknown) => void,
): ReplicaWriter<T> {
  let producer: (() => T | null) | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let firstScheduledAt: number | undefined;
  let tail: Promise<void> = Promise.resolve();
  const clear = (): void => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    firstScheduledAt = undefined;
  };
  const land = (): Promise<void> => {
    clear();
    const produce = producer;
    producer = undefined;
    if (produce === undefined || !active()) return tail;
    let value: T | null;
    try {
      value = produce();
    } catch (error) {
      onError(error);
      return tail;
    }
    if (value === null) return tail;
    const next = value;
    tail = tail.then(() => active() ? cache.save(next) : undefined).catch(onError);
    return tail;
  };
  return {
    schedule: (produce, opts = {}): void => {
      if (!active()) return;
      producer = produce;
      if (opts.immediate === true) {
        void land();
        return;
      }
      const now = Date.now();
      firstScheduledAt ??= now;
      if (timer !== undefined) clearTimeout(timer);
      const remaining = Math.max(0, firstScheduledAt + timing.maxWaitMs - now);
      timer = setTimeout(() => void land(), Math.min(timing.debounceMs, remaining));
    },
    flush: (): Promise<void> => producer === undefined ? tail : land(),
    cancel: (): void => {
      clear();
      producer = undefined;
    },
  };
}

export interface SessionReplica {
  loadTail(): Promise<ReplicaTail | null>;
  scheduleTail(produce: () => ReplicaTail | null, opts?: { readonly immediate?: boolean }): void;
  loadDelivery(): Promise<ReplicaDelivery | null>;
  saveDelivery(delivery: ReplicaDelivery): Promise<void>;
  discard(): Promise<void>;
}

export interface Replica {
  loadSessions(): Promise<ReplicaSessions | null>;
  recordSessions(sessions: readonly SessionMeta[]): void;
  loadMachines(): Promise<ReplicaMachines | null>;
  recordMachines(revision: number, machines: readonly MachineSummary[]): void;
  session(sessionId: string): SessionReplica;
  /** Sessions with a cached transcript tail; empty when the store is unreadable. */
  listTailSessions(): Promise<string[]>;
  /** Drop caches for sessions the Hub no longer lists. Throttled by callers. */
  retainSessions(valid: ReadonlySet<string>): Promise<void>;
  flush(): Promise<void>;
  /** Stop scheduling writes. Pending producers are dropped. */
  seal(): void;
  /** Explicit sign-out: forget everything this device painted. */
  discardAll(): Promise<void>;
}

export function createReplica(
  db: ReplicaDatabase,
  opts: {
    readonly now?: () => number;
    readonly debounceMs?: number;
    readonly maxWaitMs?: number;
    readonly onError?: (error: unknown) => void;
  } = {},
): Replica {
  const now = opts.now ?? Date.now;
  const timing = {
    debounceMs: opts.debounceMs ?? REPLICA_WRITE_DEBOUNCE_MS,
    maxWaitMs: opts.maxWaitMs ?? REPLICA_WRITE_MAX_WAIT_MS,
  };
  const onError = opts.onError ?? ((): void => undefined);
  let sealed = false;
  const active = (): boolean => !sealed;
  const sessionsCache = db.cache<unknown>({ kind: "service", state: "sessions" });
  const machinesCache = db.cache<unknown>({ kind: "service", state: "machines" });
  const sessionsWriter = createWriter<ReplicaSessions>(
    sessionsCache as ProductCache<ReplicaSessions>,
    timing,
    active,
    onError,
  );
  const machinesWriter = createWriter<ReplicaMachines>(
    machinesCache as ProductCache<ReplicaMachines>,
    timing,
    active,
    onError,
  );
  const sessions = new Map<string, { replica: SessionReplica; writer: ReplicaWriter<ReplicaTail> }>();
  // A load that fails is simply "nothing cached": the socket remains the
  // authority and the next broadcast repopulates the cache. It is not reported
  // through `onError`, because the common cause (no authenticated principal yet
  // while the login page is showing) is not a persistence problem.
  const decodeWith = async <T>(
    cache: ProductCache<unknown>,
    decode: (value: unknown) => T | null,
  ): Promise<T | null> => {
    try {
      const value = await cache.load();
      return value === null ? null : decode(value);
    } catch {
      return null;
    }
  };
  const session = (sessionId: string): SessionReplica => {
    if (!SESSION_ID.test(sessionId)) throw new Error("invalid session id");
    const existing = sessions.get(sessionId);
    if (existing !== undefined) return existing.replica;
    const tailCache = db.cache<unknown>({ kind: "session", session: sessionId, state: "tail" });
    const deliveryCache = db.cache<unknown>({ kind: "session", session: sessionId, state: "delivery" });
    const writer = createWriter<ReplicaTail>(
      tailCache as ProductCache<ReplicaTail>,
      timing,
      active,
      onError,
    );
    const replica: SessionReplica = {
      loadTail: () => decodeWith(tailCache, decodeReplicaTail),
      scheduleTail: (produce, scheduleOpts) => writer.schedule(produce, scheduleOpts),
      loadDelivery: () => decodeWith(deliveryCache, decodeReplicaDelivery),
      saveDelivery: async (delivery) => {
        if (sealed) return;
        try {
          await (deliveryCache as ProductCache<ReplicaDelivery>).save(delivery);
        } catch (error) {
          onError(error);
        }
      },
      discard: async () => {
        writer.cancel();
        sessions.delete(sessionId);
        const outcomes = await Promise.allSettled([tailCache.discard(), deliveryCache.discard()]);
        for (const outcome of outcomes) {
          if (outcome.status === "rejected") onError(outcome.reason);
        }
      },
    };
    sessions.set(sessionId, { replica, writer });
    return replica;
  };
  return {
    loadSessions: () => decodeWith(sessionsCache, decodeReplicaSessions),
    recordSessions: (list): void => {
      const snapshot = [...list];
      sessionsWriter.schedule(() => ({ receivedAt: now(), sessions: snapshot }));
    },
    loadMachines: () => decodeWith(machinesCache, decodeReplicaMachines),
    recordMachines: (revision, machines): void => {
      const snapshot = [...machines];
      machinesWriter.schedule(() => ({ receivedAt: now(), revision, machines: snapshot }));
    },
    session,
    listTailSessions: async (): Promise<string[]> => {
      if (sealed) return [];
      try {
        return await db.cacheSessions("tail");
      } catch {
        return [];
      }
    },
    retainSessions: async (valid): Promise<void> => {
      if (sealed) return;
      let stored: string[];
      try {
        const [tails, deliveries] = await Promise.all([
          db.cacheSessions("tail"),
          db.cacheSessions("delivery"),
        ]);
        stored = [...new Set([...tails, ...deliveries])];
      } catch (error) {
        onError(error);
        return;
      }
      if (sealed) return;
      await Promise.all(
        stored.filter((id) => !valid.has(id)).map((id) => session(id).discard()),
      );
    },
    flush: async (): Promise<void> => {
      await Promise.all([
        sessionsWriter.flush(),
        machinesWriter.flush(),
        ...[...sessions.values()].map((entry) => entry.writer.flush()),
      ]);
    },
    seal: (): void => {
      sealed = true;
      sessionsWriter.cancel();
      machinesWriter.cancel();
      for (const entry of sessions.values()) entry.writer.cancel();
    },
    discardAll: async (): Promise<void> => {
      sealed = true;
      sessionsWriter.cancel();
      machinesWriter.cancel();
      for (const entry of sessions.values()) entry.writer.cancel();
      sessions.clear();
      await db.discardCaches();
    },
  };
}
