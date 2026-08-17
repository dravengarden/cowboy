import { assertEquals } from "jsr:@std/assert";
import {
  canReturnFromPendingRow,
  destinationForPrompt,
  firstDeliveryAttempt,
  homeForOrigin,
  pendingSyncAppearance,
  retryDeliveryAttempt,
  returnLabelForHome,
  statusAfterExplicitSend,
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
    status: "pending",
    armConfirmationTimeout: true,
  });
});

Deno.test("an explicit retry that still has no network becomes a failure", () => {
  assertEquals(retryDeliveryAttempt(false), {
    status: "failed",
    armConfirmationTimeout: false,
  });
  assertEquals(retryDeliveryAttempt(true), {
    status: "pending",
    armConfirmationTimeout: true,
  });
});

Deno.test("offline unconfirmed rows show syncing chrome; connected pending stays quiet", () => {
  assertEquals(pendingSyncAppearance(undefined, false), "hidden");
  assertEquals(pendingSyncAppearance("pending", true), "hidden");
  assertEquals(pendingSyncAppearance("pending", false), "syncing");
  assertEquals(pendingSyncAppearance("sending", false), "sending");
  assertEquals(pendingSyncAppearance("sending", true), "sending");
  assertEquals(pendingSyncAppearance("failed", true), "failed");
});

Deno.test("an explicit send paints loading as soon as the frame leaves", () => {
  assertEquals(statusAfterExplicitSend(true), "sending");
  assertEquals(statusAfterExplicitSend(false), "pending");
});

Deno.test("idle connected prompts go to the transcript; everything else is durable queue", () => {
  assertEquals(destinationForPrompt(true, true, true), "transcript");
  assertEquals(destinationForPrompt(false, true, true), "queue");
  assertEquals(destinationForPrompt(true, false, true), "queue");
  assertEquals(destinationForPrompt(true, true, false), "queue");
});

Deno.test("return is offered on queue cards and on drafts that came from the queue", () => {
  assertEquals(canReturnFromPendingRow("queued", "composer"), true);
  assertEquals(canReturnFromPendingRow("queued", "draft"), true);
  assertEquals(canReturnFromPendingRow("draft", "queue"), true);
  assertEquals(canReturnFromPendingRow("draft", "composer"), false);
  assertEquals(canReturnFromPendingRow("draft", "draft"), false);
});
