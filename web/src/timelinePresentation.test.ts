import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import type { Envelope } from "./protocol.ts";
import {
  advanceTimelinePresentation,
  revealHistoryPrepend,
} from "./timelinePresentation.ts";

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

Deno.test("history prepend is revealed without releasing a frozen live tail", () => {
  const current = [chunk(30, "shown"), chunk(40, "frozen tail")];
  const latest = [
    chunk(10, "older a"),
    chunk(20, "older b"),
    chunk(30, "canonical replacement"),
    chunk(40, "new live text"),
    chunk(50, "new live event"),
  ];

  const revealed = revealHistoryPrepend(current, latest);
  assertEquals(revealed.map((entry) => entry.seq), [10, 20, 30, 40]);
  assertStrictEquals(revealed[2], current[0]);
  assertStrictEquals(revealed[3], current[1]);

  const secondPage = revealHistoryPrepend(revealed, [
    chunk(1, "oldest"),
    ...latest,
  ]);
  assertEquals(secondPage.map((entry) => entry.seq), [1, 10, 20, 30, 40]);
  assertStrictEquals(secondPage[1], revealed[0]);
  assertStrictEquals(secondPage[4], revealed[3]);
});

Deno.test("append-only and unrelated timelines remain frozen", () => {
  const current = [chunk(10, "shown")];
  assertStrictEquals(
    revealHistoryPrepend(current, [...current, chunk(20, "live")]),
    current,
  );
  assertStrictEquals(
    revealHistoryPrepend(current, [chunk(5, "other"), chunk(11, "other")]),
    current,
  );
});
