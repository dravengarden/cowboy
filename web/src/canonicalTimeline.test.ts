import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import type { Envelope } from "./protocol.ts";
import {
  dropEventsBefore,
  lastContextClearSeq,
  mergeCanonicalTimeline,
  snapshotJoinGap,
} from "./canonicalTimeline.ts";

function message(seq: number, text: string): Envelope {
  return {
    session_id: "session",
    seq,
    kind: "update",
    update: {
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text },
    },
  };
}

Deno.test("canonical history repairs an equal-sequence live message", () => {
  const prefix = message(1, "question");
  const replayCorrupted = message(2, "answer answer");
  const canonical = message(2, "answer");
  const suffix = message(3, "next");

  const merged = mergeCanonicalTimeline(
    [prefix, replayCorrupted, suffix],
    [canonical],
  );

  assertEquals(merged.map((event) => event.seq), [1, 2, 3]);
  assertStrictEquals(merged[0], prefix);
  assertStrictEquals(merged[1], canonical);
  assertStrictEquals(merged[2], suffix);
});

Deno.test("reconnect snapshot that overlaps the kept prefix needs no gap fill", () => {
  assertEquals(
    snapshotJoinGap(
      [message(1, "old"), message(80, "recent")],
      [message(80, "recent"), message(90, "tail")],
    ),
    null,
  );
});

Deno.test("a reconnect tail that does not overlap the kept prefix is a middle hole", () => {
  const prefix = message(5249, "answer");
  const tail = message(5953, "read");
  assertEquals(
    snapshotJoinGap([prefix], [tail, message(5960, "later")]),
    { beforeSeq: 5953, untilSeq: 5249 },
  );
  const merged = mergeCanonicalTimeline([prefix], [tail]);
  assertEquals(merged.map((event) => event.seq), [5249, 5953]);
});

Deno.test("an empty or older incoming window is not a join hole", () => {
  assertEquals(snapshotJoinGap([], [message(10, "tail")]), null);
  assertEquals(snapshotJoinGap([message(10, "kept")], []), null);
  assertEquals(
    snapshotJoinGap(
      [message(80, "kept")],
      [message(10, "older"), message(20, "older-tail")],
    ),
    null,
  );
});

Deno.test("the newest clear boundary in a run is found from the tail", () => {
  const cleared = (seq: number): Envelope => ({
    session_id: "session",
    seq,
    kind: "update",
    update: { sessionUpdate: "context_cleared", at: seq },
  });
  assertEquals(lastContextClearSeq([]), null);
  assertEquals(lastContextClearSeq([message(1, "a"), message(2, "b")]), null);
  assertEquals(
    lastContextClearSeq([
      message(1, "a"),
      cleared(2),
      message(3, "b"),
      cleared(4),
      message(5, "c"),
    ]),
    4,
  );
});

Deno.test("dropping rows before a boundary keeps identity when nothing is older", () => {
  const run = [message(4, "kept"), message(5, "tail")];
  assertStrictEquals(dropEventsBefore(run, 4), run);
  assertStrictEquals(dropEventsBefore(run, 1), run);
  assertEquals(dropEventsBefore(run, 5).map((event) => event.seq), [5]);
  assertEquals(dropEventsBefore(run, 6), []);
  assertEquals(dropEventsBefore([], 3), []);
});
