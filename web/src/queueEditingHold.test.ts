import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import type { Inbound } from "./protocol.ts";
import {
  getQueueEditHoldId,
  getQueueEditHoldStatus,
  getRemoteQueueEditing,
  markQueueEditHoldsUnconfirmed,
  replayQueueEditHolds,
  setQueueEditHold,
  syncRemoteQueueEditing,
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
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  assertEquals(sent, [{ type: "set_queue_editing", session_id: "s1", id: "q-1" }]);
  syncRemoteQueueEditing("s1", "q-1");
  assertEquals(getQueueEditHoldStatus("s1"), "confirmed");

  online = false;
  markQueueEditHoldsUnconfirmed();
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");

  online = true;
  replayQueueEditHolds(send);
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  assertEquals(sent.length, 2);
  // Another terminal superseding the hold clears confirmation and requires a
  // fresh acquire before a later echo can confirm again.
  syncRemoteQueueEditing("s1", "q-other");
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  syncRemoteQueueEditing("s1", "q-1");
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  setQueueEditHold("s1", "q-1", send);
  syncRemoteQueueEditing("s1", "q-1");
  assertEquals(getQueueEditHoldStatus("s1"), "confirmed");

  setQueueEditHold("s1", null, send);
  assertEquals(getQueueEditHoldId("s1"), null);
  assertEquals(getQueueEditHoldStatus("s1"), "none");
  assertEquals(sent.at(-1), { type: "set_queue_editing", session_id: "s1", id: null });

  replayQueueEditHolds(send);
  assertEquals(sent.length, 4);
});

test("remote editing broadcast updates status and never confirms an unsent hold", () => {
  const offline = (): boolean => false;
  setQueueEditHold("s2", "q-9", offline);
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");
  // Another terminal holding q-9 must not confirm our unsent offline hold.
  syncRemoteQueueEditing("s2", "q-9");
  assertEquals(getRemoteQueueEditing("s2"), "q-9");
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");

  // Once this socket transmits its acquire, the server echo confirms it.
  setQueueEditHold("s2", "q-9", () => true);
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");
  syncRemoteQueueEditing("s2", "q-9");
  assertEquals(getQueueEditHoldStatus("s2"), "confirmed");
  syncRemoteQueueEditing("s2", null);
  assertEquals(getRemoteQueueEditing("s2"), null);
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");
  // A later echo for q-9 without a fresh acquire must stay unconfirmed.
  syncRemoteQueueEditing("s2", "q-9");
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");
  setQueueEditHold("s2", null, () => true);
});
