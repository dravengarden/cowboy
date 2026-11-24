import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import type { Inbound } from "./protocol.ts";
import {
  getQueueEditHoldId,
  getQueueEditHoldStatus,
  markQueueEditHoldsUnconfirmed,
  replayQueueEditHolds,
  setQueueEditHold,
} from "./queueEditingHold.ts";

test("queue edit hold tracks offline, reconnect replay, and release", () => {
  const sent: Inbound[] = [];
  let online = false;
  const send = (cmd: Inbound): boolean => {
    if (!online) return false;
    sent.push(cmd);
    return true;
  };

  setQueueEditHold("s1", "q-1", send);
  assertEquals(getQueueEditHoldId("s1"), "q-1");
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  assertEquals(sent.length, 0);

  online = true;
  replayQueueEditHolds(send);
  assertEquals(getQueueEditHoldStatus("s1"), "confirmed");
  assertEquals(sent, [{ type: "set_queue_editing", session_id: "s1", id: "q-1" }]);

  online = false;
  markQueueEditHoldsUnconfirmed();
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");

  online = true;
  replayQueueEditHolds(send);
  assertEquals(getQueueEditHoldStatus("s1"), "confirmed");
  assertEquals(sent.length, 2);

  setQueueEditHold("s1", null, send);
  assertEquals(getQueueEditHoldId("s1"), null);
  assertEquals(getQueueEditHoldStatus("s1"), "none");
  assertEquals(sent.at(-1), { type: "set_queue_editing", session_id: "s1", id: null });

  replayQueueEditHolds(send);
  assertEquals(sent.length, 3);
});
