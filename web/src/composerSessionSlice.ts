import { contextStaleAfterCompaction, recordedContextUsageAt } from "./contextUsageRecord";
import type { SessionMeta } from "./protocol";

export interface ComposerDestination {
  id: string;
  title: string;
  cwd: string;
}

/** Session-list data that visibly affects Composer. Other sessions frequently
 * change status/usage while agents run; those broadcasts must not rerender the
 * active editor merely because the draft destination picker shares the list. */
export interface ComposerSessionSlice {
  provider: string;
  providerVersion: string | undefined;
  providerDigest: string | undefined;
  paused: boolean;
  wrapUpSteering: boolean;
  contextUsed: number;
  contextSize: number;
  /** A completed compaction left the reading as history. */
  contextStale: boolean;
  /** When the agent measured the reading; 0 when unknown. */
  contextObservedAtMs: number;
  /** Set while the reading is a persisted one from a stopped worker. */
  contextRecordedAtMs: number | undefined;
  destinations: ComposerDestination[];
}

const CACHE = new WeakMap<SessionMeta[], Map<string, ComposerSessionSlice>>();

export function composerSessionSlice(
  sessions: SessionMeta[],
  sessionId: string,
): ComposerSessionSlice {
  let bySession = CACHE.get(sessions);
  if (!bySession) {
    bySession = new Map();
    CACHE.set(sessions, bySession);
  }
  const cached = bySession.get(sessionId);
  if (cached) return cached;

  const active = sessions.find((session) => session.id === sessionId);
  const slice: ComposerSessionSlice = {
    provider: active?.provider ?? "",
    providerVersion: active?.provider_version,
    providerDigest: active?.provider_generation_digest,
    paused: active?.paused ?? false,
    wrapUpSteering: active?.wrap_up_steering === true,
    contextUsed: active?.context_used ?? 0,
    contextSize: active?.context_size ?? 0,
    contextStale: contextStaleAfterCompaction(active),
    contextObservedAtMs: active?.usage?.observed_at_ms ?? 0,
    contextRecordedAtMs: recordedContextUsageAt(active),
    destinations: sessions
      .filter((session) => session.id !== sessionId)
      .map(({ id, title, cwd }) => ({ id, title, cwd })),
  };
  bySession.set(sessionId, slice);
  return slice;
}

/** The broadcast reading `useCompactionContext` settles a refresh against. */
export function composerServerReading(slice: ComposerSessionSlice): {
  serverUsed: number;
  serverSize: number;
  serverObservedAt: number;
  serverStale: boolean;
} {
  return {
    serverUsed: slice.contextUsed,
    serverSize: slice.contextSize,
    serverObservedAt: slice.contextObservedAtMs,
    serverStale: slice.contextStale,
  };
}

export function sameComposerSessionSlice(
  a: ComposerSessionSlice,
  b: ComposerSessionSlice,
): boolean {
  return (
    a === b ||
    (a.provider === b.provider &&
      a.providerVersion === b.providerVersion &&
      a.providerDigest === b.providerDigest &&
      a.paused === b.paused &&
      a.wrapUpSteering === b.wrapUpSteering &&
      a.contextUsed === b.contextUsed &&
      a.contextSize === b.contextSize &&
      a.contextStale === b.contextStale &&
      a.contextObservedAtMs === b.contextObservedAtMs &&
      a.contextRecordedAtMs === b.contextRecordedAtMs &&
      a.destinations.length === b.destinations.length &&
      a.destinations.every((destination, index) => {
        const other = b.destinations[index];
        return (
          other !== undefined &&
          destination.id === other.id &&
          destination.title === other.title &&
          destination.cwd === other.cwd
        );
      }))
  );
}

/** Equality for the navbar's normally-closed session settings sheet.
 * Scheduling and other row-only metadata do not appear there; its Context
 * row shows the reading and whether a compaction left it as history. */
export function sameComposerSheetSession(
  a: SessionMeta | undefined,
  b: SessionMeta | undefined,
): boolean {
  return (
    a === b ||
    (a !== undefined &&
      b !== undefined &&
      a.id === b.id &&
      a.provider === b.provider &&
      a.provider_version === b.provider_version &&
      a.provider_generation_digest === b.provider_generation_digest &&
      a.cwd === b.cwd &&
      a.title === b.title &&
      a.status === b.status &&
      a.origin === b.origin &&
      a.paused === b.paused &&
      a.context_used === b.context_used &&
      a.context_size === b.context_size &&
      contextStaleAfterCompaction(a) === contextStaleAfterCompaction(b) &&
      sameProviderUpdateOffer(a, b))
  );
}

/** The update offer drives the session-settings badge and card. Its
 *  automatic time is re-estimated on every broadcast; only a minute-level
 *  change is visible. */
function sameProviderUpdateOffer(a: SessionMeta, b: SessionMeta): boolean {
  const x = a.provider_update_available;
  const y = b.provider_update_available;
  const minute = (at: number | undefined): number | undefined =>
    at === undefined ? undefined : Math.round(at / 60_000);
  return (
    a.system === b.system &&
    (a.provider_update === undefined) === (b.provider_update === undefined) &&
    (x === y ||
      (x !== undefined &&
        y !== undefined &&
        x.version === y.version &&
        x.digest === y.digest &&
        x.when_idle === y.when_idle &&
        x.on_open === y.on_open &&
        minute(x.automatic_at_ms) === minute(y.automatic_at_ms)))
  );
}
