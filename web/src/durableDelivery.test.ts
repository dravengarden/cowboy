import { assertEquals } from "jsr:@std/assert";
import {
  durableDeliveryAttempt,
  sendAfterDurableSnapshot,
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
    status: "sending",
    armConfirmationTimeout: true,
  });
});

Deno.test("a reconnect window routes even an idle prompt through the durable outbox", () => {
  assertEquals(shouldUseTranscriptDelivery(false, true, true), false);
  assertEquals(shouldUseTranscriptDelivery(true, true, true), true);
  assertEquals(shouldUseTranscriptDelivery(true, false, true), false);
  assertEquals(shouldUseTranscriptDelivery(true, true, false), false);
});

Deno.test("a dependent send waits for its latest durable snapshot", async () => {
  const barrier = Promise.withResolvers<void>();
  let sent = 0;
  const sending = sendAfterDurableSnapshot({
    flush: () => barrier.promise,
    pending: () => [{ id: "send-draft" }],
  }, "send-draft", () => sent++);
  await Promise.resolve();
  assertEquals(sent, 0);
  barrier.resolve();
  await sending;
  assertEquals(sent, 1);
});

Deno.test("discard during the source-ack commit prevents the pending send", async () => {
  const barrier = Promise.withResolvers<void>();
  let pending = [{ id: "send-draft" }];
  let sent = false;
  const sending = sendAfterDurableSnapshot({
    flush: () => barrier.promise,
    pending: () => pending,
  }, "send-draft", () => { sent = true; });
  pending = [];
  barrier.resolve();
  await sending;
  assertEquals(sent, false);
});

Deno.test("a failed source-ack commit never sends the dependent activation", async () => {
  let sent = false;
  const result = await sendAfterDurableSnapshot({
    flush: () => Promise.reject(new Error("disk unavailable")),
    pending: () => [{ id: "send-draft" }],
  }, "send-draft", () => { sent = true; }).then(() => "ok", (error: Error) => error.message);
  assertEquals(result, "disk unavailable");
  assertEquals(sent, false);
});
