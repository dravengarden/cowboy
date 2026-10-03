import { assertEquals, assertRejects } from "jsr:@std/assert";
import {
  createAuthoredSendGate,
  type PendingAuthoredSend,
} from "./authoredSendGate.ts";

const held = (id: string): PendingAuthoredSend => ({
  id,
  authored: true,
  held: true,
});

Deno.test("sending waits for restored obligations before asking or consuming its source", async () => {
  const restoration = Promise.withResolvers<void>();
  let pending: PendingAuthoredSend[] = [];
  let decisions = 0;
  let consumed = false;
  const gate = createAuthoredSendGate({
    hydrate: () => restoration.promise,
    pending: () => pending,
    decide: () => {
      decisions++;
      return Promise.reject(new DOMException("Keep source", "AbortError"));
    },
  });
  const send = gate.prepare("session").then(() => {
    consumed = true;
  });
  const rejection = assertRejects(() => send, DOMException, "Keep source");
  await Promise.resolve();
  assertEquals(decisions, 0);
  pending = [held("restored")];
  restoration.resolve();
  await rejection;
  assertEquals(decisions, 1);
  assertEquals(consumed, false);
});

Deno.test("restoration failure leaves the source untouched and opens no decision", async () => {
  let decisions = 0;
  const gate = createAuthoredSendGate({
    hydrate: () => Promise.reject(new Error("Storage unavailable")),
    pending: () => [held("old")],
    decide: async () => {
      decisions++;
    },
  });
  await assertRejects(
    () => gate.prepare("session"),
    Error,
    "Storage unavailable",
  );
  assertEquals(decisions, 0);
});

Deno.test("explicit processing excludes its own retry identity but still checks other authored sends", async () => {
  let pending: PendingAuthoredSend[] = [
    held("source"),
    { ...held("source-operation"), sourceCmid: "source" },
    { ...held("metadata"), authored: false },
    { ...held("sending"), held: false },
    held("other"),
  ];
  const captured: string[][] = [];
  const gate = createAuthoredSendGate({
    hydrate: async () => {},
    pending: () => pending,
    decide: async (sessionId, ids) => {
      assertEquals(sessionId, "session");
      captured.push([...ids]);
      pending = pending.filter((row) => !ids.includes(row.id));
    },
  });
  await gate.prepare("session", "source");
  assertEquals(captured, [["other"]]);
  assertEquals(pending.length, 4);
});

Deno.test("a new held send arriving during a decision needs its own decision", async () => {
  let pending = [held("first")];
  const decisions: string[][] = [];
  const gate = createAuthoredSendGate({
    hydrate: async () => {},
    pending: () => pending,
    decide: async (_sessionId, ids) => {
      decisions.push([...ids]);
      pending = decisions.length === 1 ? [held("arrived-while-waiting")] : [];
    },
  });
  await gate.prepare("session");
  assertEquals(decisions, [["first"], ["arrived-while-waiting"]]);
});

Deno.test("retrying is still pending until the original obligation is confirmed", () => {
  let pending: PendingAuthoredSend[] = [{ ...held("original"), held: false }];
  const gate = createAuthoredSendGate({
    hydrate: async () => {},
    pending: () => pending,
  });
  assertEquals(gate.hasPending("session", ["original"]), true);
  pending = [];
  assertEquals(gate.hasPending("session", ["original"]), false);
});
