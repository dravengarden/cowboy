// Background transcript prefetch (docs/offline-first-sync.md §Architecture 3,
// priority P2). Pure policy plus a small runner; the store owns the fetches.
//
// After the active session's bootstrap settles, the tails of sessions the user
// is likely to switch to are fetched in the background so a later switch, or a
// later offline open, paints from cache. Busy sessions come first: they are
// the ones changing. The transcript MRU follows, newest first. Nothing here
// gates a send, and a session switch cancels everything queued.

import type { Status } from "./protocol.ts";

export interface PrefetchCandidateInput {
  readonly sessions: readonly { readonly id: string; readonly status: Status }[];
  /** The opened session: its bootstrap is P1 and never a prefetch. */
  readonly activeId: string | undefined;
  /** Transcript MRU, oldest first (the store's `transcriptSessionCache`). */
  readonly recent: readonly string[];
  readonly hydrated: ReadonlySet<string>;
  /** Upper bound so prefetches never evict the active session from the MRU. */
  readonly limit: number;
}

/** Sessions worth fetching now, in priority order, capped at `limit`. */
export function prefetchCandidates(input: PrefetchCandidateInput): string[] {
  const listed = new Set(input.sessions.map((session) => session.id));
  const out: string[] = [];
  const push = (id: string): void => {
    if (id === input.activeId || input.hydrated.has(id) || !listed.has(id)) return;
    if (!out.includes(id)) out.push(id);
  };
  for (const session of input.sessions) {
    if (session.status === "busy") push(session.id);
  }
  for (let index = input.recent.length - 1; index >= 0; index -= 1) {
    push(input.recent[index]!);
  }
  return out.slice(0, Math.max(0, input.limit));
}

export interface PrefetchRunner {
  /** Replace the queue with `ids`; sessions already in flight are kept. */
  schedule(ids: readonly string[]): void;
  /** Drop the queue and abort every fetch in flight (a switch preempts P2).
   * `except` keeps one fetch running: the session being opened may already
   * be in flight as a prefetch, and its bootstrap must not be thrown away. */
  cancel(except?: string): void;
  readonly inFlight: readonly string[];
  readonly queued: readonly string[];
}

export function createPrefetchRunner(opts: {
  readonly concurrency: number;
  readonly start: (sessionId: string) => Promise<void>;
  readonly abort: (sessionId: string) => void;
}): PrefetchRunner {
  let queue: string[] = [];
  const inFlight = new Set<string>();
  let generation = 0;
  const pump = (): void => {
    while (inFlight.size < opts.concurrency && queue.length > 0) {
      const id = queue.shift()!;
      if (inFlight.has(id)) continue;
      inFlight.add(id);
      const started = generation;
      let settled: Promise<void>;
      try {
        settled = opts.start(id);
      } catch {
        settled = Promise.resolve();
      }
      void settled.catch(() => undefined).then(() => {
        inFlight.delete(id);
        if (started === generation) pump();
      });
    }
  };
  return {
    schedule(ids): void {
      queue = ids.filter((id) => !inFlight.has(id));
      pump();
    },
    cancel(except?: string): void {
      generation += 1;
      queue = [];
      for (const id of inFlight) {
        if (id !== except) opts.abort(id);
      }
    },
    get inFlight(): readonly string[] {
      return [...inFlight];
    },
    get queued(): readonly string[] {
      return [...queue];
    },
  };
}
