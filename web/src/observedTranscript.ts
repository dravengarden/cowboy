// A subagent's observed full transcript (docs/subagents.md#observed-transcripts):
// pages of the child's own ACP updates the Controller stored apart from the
// parent timeline, merged into Envelopes the shared Transcript renders.
import type { AcpUpdate, Envelope } from "./protocol";

export interface ObservedEvent {
  seq: number;
  ts_ms: number;
  update: AcpUpdate;
}

export interface ObservedPage {
  events: ObservedEvent[];
  more_before: boolean;
  more_after?: boolean;
}

export interface ObservedTranscript {
  /** Ascending, unique by seq. */
  events: ObservedEvent[];
  moreBefore: boolean;
  /** Observed updates the Controller did not keep (dropped or over limit). */
  gaps: number;
}

export const GAP_UPDATE = "cowboy_observation_gap";

export const EMPTY_TRANSCRIPT: ObservedTranscript = { events: [], moreBefore: false, gaps: 0 };

export type TranscriptTransport = (
  sessionId: string,
  childId: string,
  cursor: { after?: number; before?: number },
) => Promise<ObservedPage>;

async function defaultTransport(
  sessionId: string,
  childId: string,
  cursor: { after?: number; before?: number },
): Promise<ObservedPage> {
  const params = new URLSearchParams();
  if (cursor.after !== undefined) params.set("after_seq", String(cursor.after));
  if (cursor.before !== undefined) params.set("before_seq", String(cursor.before));
  const query = params.size > 0 ? `?${params.toString()}` : "";
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(sessionId)}/subagents/${encodeURIComponent(childId)}/transcript${query}`,
    { cache: "no-store" },
  );
  if (!response.ok) throw new Error(`subagent transcript failed (${String(response.status)})`);
  const body = (await response.json()) as Partial<ObservedPage>;
  return {
    events: Array.isArray(body.events) ? body.events : [],
    more_before: body.more_before === true,
    more_after: body.more_after === true,
  };
}

let transport: TranscriptTransport = defaultTransport;

/** Fixtures substitute the Controller. */
export function setSubagentTranscriptTransport(next: TranscriptTransport | null): void {
  transport = next ?? defaultTransport;
}

export function fetchObservedPage(
  sessionId: string,
  childId: string,
  cursor: { after?: number; before?: number },
): Promise<ObservedPage> {
  return transport(sessionId, childId, cursor);
}

/** Merge a page into what is shown. A coalesced message row arrives again
 *  with the same seq and replaces the earlier copy. */
export function mergeObserved(
  current: ObservedTranscript,
  page: ObservedPage,
  direction: "newest" | "after" | "before",
): ObservedTranscript {
  const bySeq = new Map(current.events.map((event) => [event.seq, event]));
  for (const event of page.events) {
    if (Number.isSafeInteger(event.seq) && event.update && typeof event.update === "object") {
      bySeq.set(event.seq, event);
    }
  }
  const events = [...bySeq.values()].sort((a, b) => a.seq - b.seq);
  // Only the first page and an older page learn whether older rows remain.
  const moreBefore =
    direction === "before" || current.events.length === 0 ? page.more_before : current.moreBefore;
  return {
    events,
    moreBefore,
    gaps: events.filter((event) => event.update.sessionUpdate === GAP_UPDATE).length,
  };
}

/** The newest seq shown, the cursor for rows still arriving. A coalesced
 *  message keeps its seq, so its row is fetched again from there. */
export function afterCursor(transcript: ObservedTranscript): number | undefined {
  const last = transcript.events.at(-1);
  if (!last) return undefined;
  return last.update.sessionUpdate === "agent_message_chunk" ||
    last.update.sessionUpdate === "agent_thought_chunk"
    ? last.seq - 1
    : last.seq;
}

/** Envelopes for the shared Transcript: the delegation prompt as the first
 *  user message (it is not part of the child's own updates), then the rows. */
export function observedTimeline(
  timelineId: string,
  transcript: ObservedTranscript,
  prompt: string | null,
): Envelope[] {
  const timeline: Envelope[] = [];
  if (prompt && !transcript.moreBefore) {
    timeline.push({
      session_id: timelineId,
      seq: 0,
      cmid: `${timelineId}:prompt`,
      kind: "update",
      update: { sessionUpdate: "user_message_chunk", content: { type: "text", text: prompt } },
    });
  }
  for (const event of transcript.events) {
    if (event.update.sessionUpdate === GAP_UPDATE) continue;
    timeline.push({ session_id: timelineId, seq: event.seq, kind: "update", update: event.update });
  }
  return timeline;
}
