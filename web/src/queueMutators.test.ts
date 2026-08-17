import { assertEquals } from "jsr:@std/assert";
import {
  appendUnique,
  emptyQueueValue,
  queueMutators,
  removeById,
  settledTransitionIds,
} from "./queueMutators.ts";

const draft = { id: "d1", text: "parked", cmid: "c-draft" };
const queued = { id: "q1", text: "next", cmid: "c-queue" };
const presented = { id: "opt-c-act", text: "parked", cmid: "c-act", origin: "draft" as const };

Deno.test("queue mutators never duplicate a row that is already present", () => {
  const once = appendUnique([draft], draft);
  assertEquals(once.length, 1);
  assertEquals(appendUnique([draft], { id: "d2", text: "other", cmid: "c-draft" }).length, 1);
  assertEquals(removeById([draft, queued], "missing"), [draft, queued]);
});

Deno.test("activating a draft lands it in the queue immediately", () => {
  const next = queueMutators.activateDraft(
    { queue: [queued], drafts: [draft], inFlight: [] },
    { id: "d1", row: presented },
  );
  assertEquals(next.drafts, []);
  assertEquals(next.queue.map((row) => row.id), ["q1", "opt-c-act"]);
});

Deno.test("activating a draft onto the transcript only hides the draft", () => {
  const next = queueMutators.activateDraft(
    { queue: [], drafts: [draft], inFlight: [] },
    { id: "d1" },
  );
  assertEquals(next.drafts, []);
  assertEquals(next.queue, []);
  assertEquals(next.inFlight, []);
});

Deno.test("sending or force-pushing a queued row parks it in-flight", () => {
  const sent = queueMutators.sendQueued(
    { queue: [queued], drafts: [], inFlight: [] },
    { id: "q1", row: { ...queued, id: "opt-send" } },
  );
  assertEquals(sent.queue, []);
  assertEquals(sent.inFlight.map((row) => row.id), ["opt-send"]);
  const forced = queueMutators.forceQueued(sent, {
    id: "q1",
    row: { ...queued, id: "opt-send" },
  });
  assertEquals(forced.inFlight.length, 1);
});

Deno.test("returning a queued row restores it as a draft without duplicating", () => {
  const next = queueMutators.returnQueuedToDraft(
    { queue: [queued], drafts: [], inFlight: [] },
    { id: "q1", row: { ...queued, origin: "queue" } },
  );
  assertEquals(next.queue, []);
  assertEquals(next.drafts.map((row) => row.id), ["q1"]);
  assertEquals(
    queueMutators.returnQueuedToDraft(next, { id: "q1", row: queued }).drafts.length,
    1,
  );
});

Deno.test("a transition is settled once the source id is gone from the server lists", () => {
  assertEquals(
    settledTransitionIds(
      [{ id: "op-1", name: "activateDraft", args: { id: "d1" } }],
      { ...emptyQueueValue(), queue: [presented] },
    ),
    ["op-1"],
  );
  assertEquals(
    settledTransitionIds(
      [{ id: "op-2", name: "sendQueued", args: { id: "q1" } }],
      { ...emptyQueueValue(), queue: [queued] },
    ),
    [],
  );
  assertEquals(
    settledTransitionIds(
      [{ id: "op-3", name: "returnQueuedToDraft", args: { id: "q1" } }],
      { ...emptyQueueValue(), drafts: [queued] },
    ),
    ["op-3"],
  );
});
