import type { Envelope } from "./protocol";

/** Merge immutable history pages for one ancestor segment, strictly enforcing
 *  an exclusive upper sequence cutoff (`seq < before`). A `before` of 0 (an
 *  empty source at handoff time) admits zero events. */
export function mergeAncestorEvents(
  existing: readonly Envelope[],
  incoming: readonly Envelope[],
  before?: number,
): Envelope[] {
  const bySeq = new Map<number, Envelope>();
  for (const env of existing) {
    if (before !== undefined && env.seq >= before) continue;
    bySeq.set(env.seq, env);
  }
  for (const env of incoming) {
    if (before !== undefined && env.seq >= before) continue;
    bySeq.set(env.seq, env);
  }
  return [...bySeq.values()].sort((left, right) => left.seq - right.seq);
}
