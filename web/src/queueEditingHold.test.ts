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

type HoldFrame = Extract<Inbound, { type: "set_queue_editing" }>;

test("queue edit hold tracks offline, reconnect replay, token matching, and release", () => {
  const sent: HoldFrame[] = [];
  let online = false;
  const send = (cmd: Inbound): boolean => {
    if (!online) return false;
    if (cmd.type === "set_queue_editing") sent.push(cmd);
    return true;
  };

  setQueueEditHold("s1", "q-1", send);
  assertEquals(getQueueEditHoldId("s1"), "q-1");
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  assertEquals(sent.length, 0);

  online = true;
  replayQueueEditHolds(send);
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  assertEquals(sent.length, 1);
  assertEquals(sent[0]?.session_id, "s1");
  assertEquals(sent[0]?.id, "q-1");
  const firstToken = sent[0]?.token;
  assertEquals(typeof firstToken, "string");

  // Another terminal holding the same row with a different token must NOT confirm.
  syncRemoteQueueEditing("s1", "q-1", "other-terminal-token");
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  // An unrelated intervening snapshot before our ack arrives must not break confirmation.
  syncRemoteQueueEditing("s1", "q-other", "other-token");
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  // Our exact token echo confirms the lease.
  syncRemoteQueueEditing("s1", "q-1", firstToken);
  assertEquals(getQueueEditHoldStatus("s1"), "confirmed");

  online = false;
  markQueueEditHoldsUnconfirmed();
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");

  online = true;
  replayQueueEditHolds(send);
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  assertEquals(sent.length, 2);
  const secondToken = sent[1]?.token;
  assertEquals(typeof secondToken, "string");
  assertEquals(secondToken !== firstToken, true);
  // Stale token from before reconnect cannot confirm the renewed hold.
  syncRemoteQueueEditing("s1", "q-1", firstToken);
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  syncRemoteQueueEditing("s1", "q-1", secondToken);
  assertEquals(getQueueEditHoldStatus("s1"), "confirmed");
  // Another terminal superseding drops confirmation until our fresh token echoes.
  syncRemoteQueueEditing("s1", "q-other", "other-token-2");
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");
  syncRemoteQueueEditing("s1", "q-1", "other-token-3");
  assertEquals(getQueueEditHoldStatus("s1"), "unconfirmed");

  setQueueEditHold("s1", null, send);
  assertEquals(getQueueEditHoldId("s1"), null);
  assertEquals(getQueueEditHoldStatus("s1"), "none");
  assertEquals(sent.at(-1), { type: "set_queue_editing", session_id: "s1", id: null });

  replayQueueEditHolds(send);
  assertEquals(sent.length, 3);
});

test("remote editing broadcast updates status and never confirms an unsent or tokenless hold", () => {
  const offline = (): boolean => false;
  setQueueEditHold("s2", "q-9", offline);
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");
  syncRemoteQueueEditing("s2", "q-9", "remote-token");
  assertEquals(getRemoteQueueEditing("s2"), "q-9");
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");

  let lastToken: string | undefined;
  setQueueEditHold("s2", "q-9", (cmd) => {
    if (cmd.type === "set_queue_editing") lastToken = cmd.token;
    return true;
  });
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");
  // Missing token never confirms.
  syncRemoteQueueEditing("s2", "q-9", null);
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");
  syncRemoteQueueEditing("s2", "q-9", lastToken);
  assertEquals(getQueueEditHoldStatus("s2"), "confirmed");
  syncRemoteQueueEditing("s2", null, null);
  assertEquals(getRemoteQueueEditing("s2"), null);
  assertEquals(getQueueEditHoldStatus("s2"), "unconfirmed");
  setQueueEditHold("s2", null, () => true);
});
