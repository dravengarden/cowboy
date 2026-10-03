import { assertEquals, assertRejects } from "jsr:@std/assert";
import {
  cancelPriorSendDecisions,
  currentPriorSendDecision,
  finishPriorSendDecision,
  requestPriorSendDecision,
  subscribePriorSendDecision,
} from "./priorSendDecision.ts";

Deno.test("prior send decisions wait for an explicit choice and serialize sessions", async () => {
  const unsubscribe = subscribePriorSendDecision(() => {});
  try {
    let proceeded = false;
    const first = requestPriorSendDecision("a", ["old-a"]).then(() => {
      proceeded = true;
    });
    const second = requestPriorSendDecision("b", ["old-b"]);
    const rejected = assertRejects(
      () => second,
      DOMException,
      "New message kept in composer",
    );
    await Promise.resolve();
    assertEquals(proceeded, false);
    assertEquals(currentPriorSendDecision()?.sessionId, "a");
    finishPriorSendDecision(currentPriorSendDecision()!, true);
    await first;
    assertEquals(proceeded, true);
    assertEquals(currentPriorSendDecision()?.sessionId, "b");
    cancelPriorSendDecisions();
    await rejected;
    assertEquals(currentPriorSendDecision(), null);
  } finally {
    cancelPriorSendDecisions();
    unsubscribe();
  }
});

Deno.test("missing decision surface refuses a new send instead of silently proceeding", async () => {
  await assertRejects(
    () => requestPriorSendDecision("a", ["old-a"]),
    Error,
    "Review the earlier message",
  );
  assertEquals(currentPriorSendDecision(), null);
});

Deno.test("concurrent decisions for the same old message have independent identities and completion", async () => {
  const unsubscribe = subscribePriorSendDecision(() => {});
  try {
    const firstSend = requestPriorSendDecision("same-session", ["same-old"]);
    const first = currentPriorSendDecision()!;
    let secondFinished = false;
    const secondSend = requestPriorSendDecision("same-session", ["same-old"])
      .then(() => {
        secondFinished = true;
      });
    finishPriorSendDecision(first, true);
    await firstSend;
    const second = currentPriorSendDecision()!;
    assertEquals(second.id === first.id, false);
    // A stale save completion must not finish the next waiting send.
    finishPriorSendDecision(first, true);
    await Promise.resolve();
    assertEquals(secondFinished, false);
    assertEquals(currentPriorSendDecision(), second);
    finishPriorSendDecision(second, true);
    await secondSend;
    assertEquals(secondFinished, true);
    assertEquals(currentPriorSendDecision(), null);
  } finally {
    cancelPriorSendDecisions();
    unsubscribe();
  }
});
