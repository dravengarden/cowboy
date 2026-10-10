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
  contextUsed: number;
  contextSize: number;
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
    contextUsed: active?.context_used ?? 0,
    contextSize: active?.context_size ?? 0,
    destinations: sessions
      .filter((session) => session.id !== sessionId)
      .map(({ id, title, cwd }) => ({ id, title, cwd })),
  };
  bySession.set(sessionId, slice);
  return slice;
}

export function sameComposerSessionSlice(
  a: ComposerSessionSlice,
  b: ComposerSessionSlice,
): boolean {
  return a === b ||
    (a.provider === b.provider &&
      a.providerVersion === b.providerVersion &&
      a.providerDigest === b.providerDigest &&
      a.paused === b.paused &&
      a.contextUsed === b.contextUsed &&
      a.contextSize === b.contextSize &&
      a.destinations.length === b.destinations.length &&
      a.destinations.every((destination, index) => {
        const other = b.destinations[index];
        return other !== undefined && destination.id === other.id &&
          destination.title === other.title && destination.cwd === other.cwd;
      }));
}

/** Equality for the navbar's normally-closed session settings sheet. Usage,
 * scheduling and other row-only metadata do not appear there. */
export function sameComposerSheetSession(
  a: SessionMeta | undefined,
  b: SessionMeta | undefined,
): boolean {
  return a === b ||
    (a !== undefined && b !== undefined &&
      a.id === b.id &&
      a.provider === b.provider &&
      a.provider_version === b.provider_version &&
      a.provider_generation_digest === b.provider_generation_digest &&
      a.cwd === b.cwd &&
      a.title === b.title &&
      a.status === b.status &&
      a.origin === b.origin &&
      a.paused === b.paused &&
      sameProviderUpdateOffer(a, b));
}

/** The update offer drives the session-settings badge and card. Its
 *  automatic time is re-estimated on every broadcast; only a minute-level
 *  change is visible. */
function sameProviderUpdateOffer(a: SessionMeta, b: SessionMeta): boolean {
  const x = a.provider_update_available;
  const y = b.provider_update_available;
  const minute = (at: number | undefined): number | undefined =>
    at === undefined ? undefined : Math.round(at / 60_000);
  return a.system === b.system &&
    (a.provider_update === undefined) === (b.provider_update === undefined) &&
    (x === y ||
      (x !== undefined && y !== undefined &&
        x.version === y.version &&
        x.digest === y.digest &&
        x.when_idle === y.when_idle &&
        minute(x.automatic_at_ms) === minute(y.automatic_at_ms)));
}
