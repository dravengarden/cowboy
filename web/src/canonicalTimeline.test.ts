import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import type { Envelope } from "./protocol.ts";
import { mergeCanonicalTimeline } from "./canonicalTimeline.ts";

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
