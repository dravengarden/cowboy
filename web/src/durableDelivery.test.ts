import { assertEquals } from "jsr:@std/assert";
import {
  durableDeliveryAttempt,
  shouldUseTranscriptDelivery,
} from "./durableDelivery.ts";

Deno.test("disconnected durable delivery waits for reconnect instead of failing", () => {
  assertEquals(durableDeliveryAttempt(false), {
    status: "pending",
    armConfirmationTimeout: false,
  });
});

Deno.test("connected durable delivery starts its confirmation timeout", () => {
  assertEquals(durableDeliveryAttempt(true), {
    status: "pending",
    armConfirmationTimeout: true,
  });
});

Deno.test("a reconnect window routes even an idle prompt through the durable outbox", () => {
  assertEquals(shouldUseTranscriptDelivery(false, true, true), false);
  assertEquals(shouldUseTranscriptDelivery(true, true, true), true);
  assertEquals(shouldUseTranscriptDelivery(true, false, true), false);
  assertEquals(shouldUseTranscriptDelivery(true, true, false), false);
});
