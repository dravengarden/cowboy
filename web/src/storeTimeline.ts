import { linkTimeline } from "./derive";
import { type Envelope, isPureTerminalOutputDelta } from "./protocol";
import { mergeCanonicalTimeline } from "./canonicalTimeline";

// A canonical live envelope may absorb many raw sequence numbers. Keep those
// numbers out-of-band so reconnect snapshots can still deduplicate their raw
// overlap without retaining every large payload. Weak keys disappear with the
// timeline rows they describe.
export const absorbedSeqs = new WeakMap<Envelope, Set<number>>();

export function absorbed(previous: Envelope, replacement: Envelope, seq: number): Envelope {
  const seen = absorbedSeqs.get(previous) ?? new Set([previous.seq]);
  seen.add(seq);
  absorbedSeqs.set(replacement, seen);
  return replacement;
}

export function containsSeq(events: Envelope[], seq: number): boolean {
  const last = events[events.length - 1];
  if (last && absorbedSeqs.get(last)?.has(seq) === true) return true;
  // Live traffic is monotonic. Avoid scanning the whole visible history for
  // the overwhelmingly common append case; reconnect/history overlap falls
  // through to the full check below.
  if (last && seq > last.seq) return false;
  return events.some((event) => event.seq === seq || absorbedSeqs.get(event)?.has(seq) === true);
}

export function findLatestTool(events: Envelope[], toolId: string): number {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const candidate = events[index];
    if (
      candidate?.kind === "update" &&
      candidate.update.sessionUpdate === "tool_call" &&
      candidate.update.toolCallId === toolId
    ) return index;
  }
  return -1;
}

export function applyEnvelope(timelines: Map<string, Envelope[]>, env: Envelope): Map<string, Envelope[]> {
  const existing = timelines.get(env.session_id) ?? [];
  if (containsSeq(existing, env.seq)) return timelines; // dedup

  // Clear is a destructive transcript boundary, not an ordinary divider.
  // Replace the local history immediately; the daemon deletes the same rows
  // durably so a reload cannot restore them.
  if (
    env.kind === "update" &&
    env.update.sessionUpdate === "context_cleared"
  ) {
    const next = new Map(timelines);
    next.set(env.session_id, [env]);
    return next;
  }

  // These high-frequency frames are runtime telemetry, not transcript history.
  // The daemon persists their sequence watermark without storing a row; mirror
  // that canonical representation in the live client so long turns don't grow
  // one inert object per token update.
  if (
    env.kind === "update" &&
    (env.update.sessionUpdate === "usage_update" ||
      env.update.sessionUpdate === "session_info_update" ||
      isPureTerminalOutputDelta(env.update))
  ) {
    return timelines;
  }

  let merged: Envelope[] | undefined;
  if (env.kind === "update") {
    const updateKind = env.update.sessionUpdate;
    if (
      updateKind === "agent_message_chunk" ||
      updateKind === "agent_thought_chunk" ||
      updateKind === "user_message_chunk"
    ) {
      const last = existing[existing.length - 1];
      if (
        last?.kind === "update" &&
        last.update.sessionUpdate === updateKind &&
        last.update.messageId === env.update.messageId &&
        last.update.content?.type === "text" &&
        env.update.content?.type === "text"
      ) {
        const replacement: Envelope = {
          ...last,
          update: {
            ...last.update,
            content: {
              ...last.update.content,
              text: `${last.update.content.text ?? ""}${env.update.content.text ?? ""}`,
            },
          },
        };
        merged = linkTimeline(
          [...existing.slice(0, -1), absorbed(last, replacement, env.seq)],
          existing,
        );
      }
    } else if (updateKind === "tool_call_update") {
      const toolId = env.update.toolCallId;
      const index = toolId ? findLatestTool(existing, toolId) : -1;
      const initial = index >= 0 ? existing[index] : undefined;
      if (initial?.kind === "update") {
        const replacement: Envelope = {
          ...initial,
          update: {
            ...initial.update,
            ...env.update,
            sessionUpdate: "tool_call",
          },
        };
        merged = linkTimeline([...existing], existing);
        merged[index] = absorbed(initial, replacement, env.seq);
      }
    }
  }

  merged ??= linkTimeline(
    existing.length === 0 || env.seq > (existing[existing.length - 1]?.seq ?? 0)
      ? [...existing, env]
      : [...existing, env].sort((a, b) => a.seq - b.seq),
    existing,
  );
  const next = new Map(timelines);
  next.set(env.session_id, merged);
  return next;
}

function toolStatusRank(status: string | undefined): number {
  if (status === "completed" || status === "failed") return 2;
  if (status === "in_progress") return 1;
  return 0;
}

function liveEventAheadOfSnapshot(
  oldEvent: Envelope,
  newEvent: Envelope,
  incomingMax: number,
): boolean {
  if (oldEvent.kind !== "update" || newEvent.kind !== "update") return false;
  if (
    oldEvent.update.sessionUpdate === "tool_call" &&
    newEvent.update.sessionUpdate === "tool_call" &&
    oldEvent.update.toolCallId !== undefined &&
    oldEvent.update.toolCallId === newEvent.update.toolCallId
  ) {
    // Tool lifecycle is strictly monotone (pending -> in_progress -> terminal);
    // never let a delayed snapshot or older page downgrade a completed/running tool.
    return toolStatusRank(oldEvent.update.status) > toolStatusRank(newEvent.update.status);
  }
  const seen = absorbedSeqs.get(oldEvent);
  if (seen === undefined) return false;
  let hasBeyond = false;
  for (const seq of seen) {
    if (seq > incomingMax) {
      hasBeyond = true;
      break;
    }
  }
  if (!hasBeyond) return false;
  if (
    oldEvent.update.sessionUpdate === newEvent.update.sessionUpdate &&
    (oldEvent.update.sessionUpdate === "agent_message_chunk" ||
      oldEvent.update.sessionUpdate === "agent_thought_chunk" ||
      oldEvent.update.sessionUpdate === "user_message_chunk") &&
    oldEvent.update.messageId === newEvent.update.messageId &&
    oldEvent.update.content?.type === "text" &&
    newEvent.update.content?.type === "text"
  ) {
    const oldText = oldEvent.update.content.text ?? "";
    const newText = newEvent.update.content.text ?? "";
    return oldText.length > newText.length && oldText.startsWith(newText);
  }
  return false;
}

// Merge two seq-ordered runs in linear time while preserving object identity
// for existing events. Scrollback used to scan the whole window for every
// incoming event and then sort the combined window on every page, making
// progressively older history increasingly expensive.
export function mergeEvents(
  timelines: Map<string, Envelope[]>,
  sessionId: string,
  events: Envelope[],
): Map<string, Envelope[]> {
  const existing = timelines.get(sessionId) ?? [];
  const incomingMax = events[events.length - 1]?.seq;
  const preferExisting = incomingMax === undefined
    ? undefined
    : (oldEvent: Envelope, newEvent: Envelope): boolean =>
        liveEventAheadOfSnapshot(oldEvent, newEvent, incomingMax);
  const merged = mergeCanonicalTimeline(existing, events, preferExisting);
  if (
    merged.length === existing.length &&
    merged.every((event, index) => event === existing[index])
  ) {
    return timelines;
  }
  const next = new Map(timelines);
  next.set(sessionId, linkTimeline(merged, existing));
  return next;
}
