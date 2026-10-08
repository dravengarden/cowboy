import { assert, assertEquals } from "jsr:@std/assert";
import {
  canReturnFromPendingRow,
  COMMITTING_STALL_MS,
  CONNECTED_PENDING_STALL_MS,
  deliveryDeadlineDeferred,
  deliveryStallMs,
  destinationForPrompt,
  firstDeliveryAttempt,
  homeForOrigin,
  lateEchoRetiresRecoveryDraft,
  pendingSyncAppearance,
  recoveredSendId,
  recoveryDraftCmid,
  recoveryDraftMatchesEcho,
  retryDeliveryAttempt,
  returnLabelForHome,
  statusAfterExplicitSend,
  unconfirmedSendDisposition,
} from "./localFirstDelivery.ts";

Deno.test("failed sends return to the list they left", () => {
  assertEquals(homeForOrigin("composer"), "draft");
  assertEquals(homeForOrigin("draft"), "draft");
  assertEquals(homeForOrigin("queue"), "queue");
  assertEquals(returnLabelForHome("draft"), "Return to drafts");
  assertEquals(returnLabelForHome("queue"), "Return to queue");
});

Deno.test("a first offline attempt waits instead of failing", () => {
  assertEquals(firstDeliveryAttempt(false), {
    status: "pending",
    armConfirmationTimeout: false,
  });
  assertEquals(firstDeliveryAttempt(true), {
    status: "sending",
    armConfirmationTimeout: true,
  });
});

Deno.test("an explicit retry that still has no network becomes a failure", () => {
  assertEquals(retryDeliveryAttempt(false), {
    status: "failed",
    armConfirmationTimeout: false,
  });
  assertEquals(retryDeliveryAttempt(true), {
    status: "sending",
    armConfirmationTimeout: true,
  });
});

Deno.test("every unconfirmed phase has explicit chrome", () => {
  assertEquals(pendingSyncAppearance(undefined, false), "hidden");
  assertEquals(pendingSyncAppearance("committing", true), "saving");
  assertEquals(pendingSyncAppearance("committing", false), "saving");
  assertEquals(pendingSyncAppearance("pending", true), "syncing");
  assertEquals(pendingSyncAppearance("pending", false), "syncing");
  assertEquals(pendingSyncAppearance("sending", false), "syncing");
  assertEquals(pendingSyncAppearance("sending", true), "sending");
  assertEquals(pendingSyncAppearance("failed", true), "failed");
});

Deno.test("a connected row that never leaves the tab is given an escape", () => {
  assertEquals(deliveryStallMs("pending", true), CONNECTED_PENDING_STALL_MS);
  // Waiting IS the contract while the socket is down; a deadline here would
  // call a healthy queued prompt a failure.
  assertEquals(deliveryStallMs("pending", false), null);
});

Deno.test("a durability barrier that never settles still releases the row", () => {
  // IndexedDB has no timeout of its own: a `versionchange` blocked by another
  // tab leaves the write unresolved, and being offline changes nothing about a
  // local write.
  assertEquals(deliveryStallMs("committing", true), COMMITTING_STALL_MS);
  assertEquals(deliveryStallMs("committing", false), COMMITTING_STALL_MS);
});

Deno.test("phases with another owner get no second deadline", () => {
  // The acknowledgement timeout owns `sending`, and `failed` already carries
  // Retry / Return / Discard.
  assertEquals(deliveryStallMs("sending", true), null);
  assertEquals(deliveryStallMs("sending", false), null);
  assertEquals(deliveryStallMs("failed", true), null);
  assertEquals(deliveryStallMs(undefined, true), null);
});

Deno.test("the stall deadline outlasts an ordinary reconnect", () => {
  // The reconnect replay is the primary recovery; the deadline only catches the
  // row that replay could not carry, so it must not preempt it.
  assert(CONNECTED_PENDING_STALL_MS > 10_000);
});

Deno.test("an explicit send paints loading as soon as the frame leaves", () => {
  assertEquals(statusAfterExplicitSend(true), "sending");
  assertEquals(statusAfterExplicitSend(false), "pending");
});

Deno.test("idle prompts go to the transcript, online or not; everything else is durable queue", () => {
  assertEquals(destinationForPrompt(true, true), "transcript");
  assertEquals(destinationForPrompt(false, true), "queue");
  assertEquals(destinationForPrompt(true, false), "queue");
});

Deno.test("return is offered on queue cards and on drafts that came from the queue", () => {
  assertEquals(canReturnFromPendingRow("queued", "composer"), true);
  assertEquals(canReturnFromPendingRow("queued", "draft"), true);
  assertEquals(canReturnFromPendingRow("draft", "queue"), true);
  assertEquals(canReturnFromPendingRow("draft", "composer"), false);
  assertEquals(canReturnFromPendingRow("draft", "draft"), false);
});

Deno.test("a timed-out transcript prompt is parked in drafts, not held as an error", () => {
  for (const phase of ["sending", "pending", "failed"] as const) {
    assertEquals(
      unconfirmedSendDisposition({
        mutation: "submitPrompt",
        phase,
        refused: false,
      }),
      "draft",
    );
  }
});

Deno.test("refusals, wedged local writes, and queue work stay held", () => {
  assertEquals(
    unconfirmedSendDisposition({
      mutation: "submitPrompt",
      phase: "sending",
      refused: true,
    }),
    "hold",
  );
  assertEquals(
    unconfirmedSendDisposition({
      mutation: "submitPrompt",
      phase: "committing",
      refused: false,
    }),
    "hold",
  );
  for (
    const mutation of [
      "addQueue",
      "frontQueue",
      "forceQueue",
      "addDraft",
      "activateDraft",
    ]
  ) {
    assertEquals(
      unconfirmedSendDisposition({
        mutation,
        phase: "sending",
        refused: false,
      }),
      "hold",
    );
  }
});

Deno.test("parking one send twice reuses the same draft", () => {
  assertEquals(recoveryDraftCmid("cmid-1"), "recovery-cmid-1");
  assertEquals(recoveryDraftCmid("cmid-1"), recoveryDraftCmid("cmid-1"));
});

Deno.test("a late echo retires only an untouched recovery draft", () => {
  const parked = { text: "iloader is ready", attachments: 0 };
  assert(
    lateEchoRetiresRecoveryDraft(parked, {
      text: "iloader is ready",
      attachments: [],
    }),
  );
  assertEquals(
    lateEchoRetiresRecoveryDraft(parked, {
      text: "iloader is ready, now pair",
      attachments: [],
    }),
    false,
  );
  assertEquals(
    lateEchoRetiresRecoveryDraft(parked, {
      text: "iloader is ready",
      attachments: [{}],
    }),
    false,
  );
});

Deno.test("a delivery deadline waits out frozen, hidden, offline and resuming time", () => {
  const settled = {
    armedAt: 0,
    delayMs: 60_000,
    now: 60_000,
    visible: true,
    connectedSince: 0,
    visibleSince: 0,
    settleMs: 10_000,
  };
  assertEquals(deliveryDeadlineDeferred(settled), false);
  // iOS runs a frozen timer on resume long after it was due.
  assertEquals(deliveryDeadlineDeferred({ ...settled, now: 600_000 }), true);
  assertEquals(deliveryDeadlineDeferred({ ...settled, visible: false }), true);
  assertEquals(deliveryDeadlineDeferred({ ...settled, connectedSince: null }), true);
  assertEquals(deliveryDeadlineDeferred({ ...settled, connectedSince: 55_000 }), true);
  assertEquals(deliveryDeadlineDeferred({ ...settled, visibleSince: 55_000 }), true);
});

Deno.test("recovery drafts map back to the parked send and match its echo", () => {
  assertEquals(recoveredSendId(recoveryDraftCmid("cmid-1")), "cmid-1");
  assertEquals(recoveredSendId("cmid-1"), null);
  assertEquals(recoveredSendId("recovery-"), null);
  assert(recoveryDraftMatchesEcho("hello\n world", "hello world"));
  assert(!recoveryDraftMatchesEcho("hello world, edited", "hello world"));
});
