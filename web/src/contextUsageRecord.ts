import type { SessionMeta } from "./protocol";
import { freshnessAge } from "./usageFreshness";

/** Statuses with no live worker to report usage: the context ring shows the
 *  reading the Controller persisted from the last worker, not a live one. */
const UNREPORTED_STATUSES: ReadonlySet<SessionMeta["status"]> = new Set([
  "starting",
  "exited",
  "crashed",
  "interrupted",
]);

/** When the shown context usage was recorded, if it is a persisted reading
 *  from a worker that is no longer running; `undefined` while it is live. */
export function recordedContextUsageAt(
  session: Pick<SessionMeta, "status" | "usage"> | undefined,
): number | undefined {
  const usage = session?.usage;
  if (!session || !usage || !(usage.size > 0)) return undefined;
  if (!UNREPORTED_STATUSES.has(session.status)) return undefined;
  return usage.observed_at_ms > 0 ? usage.observed_at_ms : undefined;
}

/** The meter's note while a completed compaction left the reading as history. */
export const COMPACTED_CONTEXT_HINT = "Compacted · updates after the next reply";

/** Whether a completed compaction made the session's context reading history:
 *  the compacted context's size is unknown until the agent reports again. */
export function contextStaleAfterCompaction(
  session: Pick<SessionMeta, "usage"> | undefined,
): boolean {
  return typeof session?.usage?.compacted_at_ms === "number";
}

/** The broadcast reading `useCompactionContext` settles a refresh against. */
export function serverContextReading(
  session: Pick<SessionMeta, "context_used" | "context_size" | "usage"> | undefined,
): { serverUsed: number; serverSize: number; serverObservedAt: number; serverStale: boolean } {
  return {
    serverUsed: session?.context_used ?? 0,
    serverSize: session?.context_size ?? 0,
    serverObservedAt: session?.usage?.observed_at_ms ?? 0,
    serverStale: contextStaleAfterCompaction(session),
  };
}

/** Suffix for a context tooltip naming a persisted reading's age. */
export function recordedContextNote(
  recordedAtMs: number | undefined,
  now: number = Date.now(),
): string {
  return recordedAtMs === undefined ? "" : ` · last recorded ${freshnessAge(recordedAtMs, now)}`;
}

// Compact "K/M" token count for the context tooltip (48436 → "48K", 1_000_000 → "1M").
function fmtTokens(n: number): string {
  if (n >= 1_000_000) {
    return `${(n / 1_000_000).toFixed(n >= 9_950_000 ? 0 : 1)}M`;
  }
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

/** The Compact tooltip — names the action and, when known, the context
 *  fullness it acts on ("Compact conversation · context 79% · 780K / 1M
 *  tokens"), plus the reading's age when no live worker reported it. */
export function compactTooltip(
  { used, size, stale = false }: { used: number; size: number; stale?: boolean },
  recordedAtMs?: number,
): string {
  if (stale) return `Compact conversation · ${COMPACTED_CONTEXT_HINT}`;
  if (!(size > 0)) return "Compact conversation";
  const pct = used > 0 ? Math.max(1, Math.round(Math.min(100, (used / size) * 100))) : 0;
  return `Compact conversation · context ${pct}% · ${fmtTokens(used)} / ${fmtTokens(
    size,
  )} tokens${recordedContextNote(recordedAtMs)}`;
}
