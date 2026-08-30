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

Deno.test("a direct prompt is retained in-flight until its user echo confirms it", () => {
  const submitted = queueMutators.submitPrompt(emptyQueueValue(), {
    row: presented,
  });
  assertEquals(submitted.queue, []);
  assertEquals(submitted.drafts, []);
  assertEquals(submitted.inFlight, [presented]);
  assertEquals(
    queueMutators.submitPrompt(submitted, { row: presented }).inFlight,
    [presented],
  );
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

Deno.test("pending edits rebase locally and settle only on matching server content", () => {
  const edited = { ...draft, text: "recovered edit" };
  const optimistic = queueMutators.editDraft(
    { ...emptyQueueValue(), drafts: [draft] },
    { id: draft.id, row: edited },
  );
  assertEquals(optimistic.drafts, [edited]);
  const pending = [{
    id: "edit-op",
    name: "editDraft",
    args: { id: draft.id, row: edited },
  }];
  assertEquals(
    settledTransitionIds(
      pending,
      { ...emptyQueueValue(), drafts: [draft] },
    ),
    [],
  );
  assertEquals(
    settledTransitionIds(
      pending,
      { ...emptyQueueValue(), drafts: [edited] },
    ),
    ["edit-op"],
  );
});

Deno.test("reschedule stays pending until the authoritative schedule matches", () => {
  const original = { ...draft, schedule: { fire_at_ms: 10, delivery: "back" } };
  const rescheduled = {
    ...original,
    schedule: { fire_at_ms: 20, delivery: "front" },
  };
  const pending = [{
    id: "schedule-op",
    name: "rescheduleDraft",
    args: { id: draft.id, row: rescheduled },
  }];
  assertEquals(
    settledTransitionIds(
      pending,
      { ...emptyQueueValue(), drafts: [original] },
    ),
    [],
  );
  assertEquals(
    settledTransitionIds(
      pending,
      { ...emptyQueueValue(), drafts: [rescheduled] },
    ),
    ["schedule-op"],
  );
});

Deno.test("remove stays pending until the authoritative row is absent", () => {
  const removed = queueMutators.removeDraft(
    { ...emptyQueueValue(), drafts: [draft] },
    { id: draft.id },
  );
  assertEquals(removed.drafts, []);
  const pending = [{
    id: "remove-op",
    name: "removeDraft",
    args: { id: draft.id },
  }];
  assertEquals(
    settledTransitionIds(pending, { ...emptyQueueValue(), drafts: [draft] }),
    [],
  );
  assertEquals(settledTransitionIds(pending, emptyQueueValue()), ["remove-op"]);
});

Deno.test("unschedule stays pending until the authoritative schedule is absent", () => {
  const scheduled = {
    ...draft,
    schedule: { fire_at_ms: 20, delivery: "back" },
  };
  const plain = { ...draft, schedule: undefined };
  const optimistic = queueMutators.unscheduleDraft(
    { ...emptyQueueValue(), drafts: [scheduled] },
    { id: draft.id, row: plain },
  );
  assertEquals(optimistic.drafts, [plain]);
  const pending = [{
    id: "unschedule-op",
    name: "unscheduleDraft",
    args: { id: draft.id, row: plain },
  }];
  assertEquals(
    settledTransitionIds(
      pending,
      { ...emptyQueueValue(), drafts: [scheduled] },
    ),
    [],
  );
  assertEquals(
    settledTransitionIds(pending, { ...emptyQueueValue(), drafts: [plain] }),
    ["unschedule-op"],
  );
});
