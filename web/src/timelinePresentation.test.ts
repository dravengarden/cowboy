import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import type { Envelope } from "./protocol.ts";
import { advanceTimelinePresentation } from "./timelinePresentation.ts";

function chunk(seq: number, text: string): Envelope {
  return {
    session_id: "s1",
    seq,
    kind: "update",
    update: {
      sessionUpdate: "agent_message_chunk",
      messageId: "m1",
      content: { type: "text", text },
    },
  };
}

function lifecycle(seq: number): Envelope {
  return {
    session_id: "s1",
    seq,
    kind: "lifecycle",
    status: "running",
    detail: null,
  };
}

Deno.test("drawer catch-up bounds growth inside a coalesced text envelope", () => {
  const current = [chunk(1, "abc")];
  const latest = [chunk(1, "abcdefghijkl")];

  const first = advanceTimelinePresentation(current, latest, 4, 2);
  assertEquals(first.timeline[0]?.kind === "update"
    ? first.timeline[0].update.content?.text
    : null, "abcdefg");
  assertEquals(first.complete, false);

  const second = advanceTimelinePresentation(first.timeline, latest, 4, 2);
  assertEquals(second.timeline[0]?.kind === "update"
    ? second.timeline[0].update.content?.text
    : null, "abcdefghijk");
  assertEquals(second.complete, false);

  const final = advanceTimelinePresentation(second.timeline, latest, 4, 2);
  assertStrictEquals(final.timeline[0], latest[0]);
  assertEquals(final.complete, true);
});

Deno.test("drawer catch-up limits appended ordinary envelopes per step", () => {
  const first = lifecycle(1);
  const latest = [first, lifecycle(2), lifecycle(3), lifecycle(4)];
  const step = advanceTimelinePresentation([first], latest, 100, 2);
  assertEquals(step.timeline.length, 3);
  assertEquals(step.complete, false);
});

Deno.test("drawer catch-up safely adopts non-append timeline replacements", () => {
  const current = [lifecycle(2)];
  const latest = [lifecycle(1), current[0]!];
  const step = advanceTimelinePresentation(current, latest);
  assertStrictEquals(step.timeline, latest);
  assertEquals(step.complete, true);
});
