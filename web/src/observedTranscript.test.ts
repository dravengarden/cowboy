import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { derive } from "./derive";
import type { AcpUpdate } from "./protocol";
import {
  afterCursor,
  EMPTY_TRANSCRIPT,
  GAP_UPDATE,
  mergeObserved,
  type ObservedEvent,
  observedTimeline,
} from "./observedTranscript";

function row(seq: number, update: Record<string, unknown>): ObservedEvent {
  return { seq, ts_ms: seq, update: update as AcpUpdate };
}

const say = (seq: number, text: string): ObservedEvent =>
  row(seq, {
    sessionUpdate: "agent_message_chunk",
    messageId: "m",
    content: { type: "text", text },
  });

test("a coalesced message row replaces its earlier copy and keeps order", () => {
  const first = mergeObserved(
    EMPTY_TRANSCRIPT,
    {
      events: [
        row(1, { sessionUpdate: "tool_call", toolCallId: "t", title: "Read" }),
        say(2, "Hel"),
      ],
      more_before: true,
    },
    "newest",
  );
  assertEquals(afterCursor(first), 1);
  const next = mergeObserved(
    first,
    {
      events: [
        say(2, "Hello"),
        row(3, { sessionUpdate: "tool_call", toolCallId: "u", title: "Edit" }),
      ],
      more_before: false,
    },
    "after",
  );
  assertEquals(
    next.events.map((event) => event.seq),
    [1, 2, 3],
  );
  assertEquals((next.events[1]?.update as { content: { text: string } }).content.text, "Hello");
  // A forward page never forgets that older rows remain.
  assertEquals(next.moreBefore, true);
  assertEquals(afterCursor(next), 3);
  const prompt = mergeObserved(
    EMPTY_TRANSCRIPT,
    {
      events: [
        row(7, { sessionUpdate: "user_message_chunk", content: { type: "text", text: "go" } }),
      ],
      more_before: false,
    },
    "newest",
  );
  // Every kind the writer extends in place is read again from its own row.
  assertEquals(afterCursor(prompt), 6);
  const older = mergeObserved(
    next,
    { events: [row(0.5, {}), say(-1, "x")], more_before: false },
    "before",
  );
  assertEquals(older.moreBefore, false);
  assertEquals(
    older.events.map((event) => event.seq),
    [-1, 1, 2, 3],
  );
});

test("the timeline starts with the delegation prompt and hides gap rows", () => {
  const transcript = mergeObserved(
    EMPTY_TRANSCRIPT,
    {
      events: [
        row(1, { sessionUpdate: GAP_UPDATE, reason: "dropped", dropped: 2 }),
        row(2, {
          sessionUpdate: "tool_call",
          toolCallId: "t",
          title: "Read file",
          status: "completed",
        }),
        say(3, "Done."),
      ],
      more_before: false,
    },
    "newest",
  );
  assertEquals(transcript.gaps, 1);
  const timeline = observedTimeline("subagent:s:c", transcript, "Find the bug");
  assertEquals(
    timeline.map((envelope) => envelope.seq),
    [0, 2, 3],
  );
  const items = derive(timeline);
  assertEquals(
    items.map((item) => item.kind),
    ["message", "tool", "message"],
  );
  // The prompt is the child's first user message only when nothing older remains.
  const partial = { ...transcript, moreBefore: true };
  assertEquals(observedTimeline("subagent:s:c", partial, "Find the bug")[0]?.seq, 2);
});
