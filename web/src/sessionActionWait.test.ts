import { test } from "bun:test";
import { assertEquals, assertRejects } from "../../tools/test-assert.ts";
import { sessionActionSnapshot, waitForSessionAction } from "./sessionActionWait";

test("reconnection sends once, only after readiness", async () => {
  let ready = false;
  let sent = 0;
  const action = waitForSessionAction({ label: "Clear", ready: () => ready,
    reconnect: () => {}, unchanged: () => true, execute: async () => { sent++; } });
  assertEquals(sessionActionSnapshot()?.phase, "waiting");
  assertEquals(sent, 0);
  ready = true;
  await action;
  assertEquals(sent, 1);
  assertEquals(sessionActionSnapshot(), null);
});

test("cancellation wins even if connection recovers before the next check", async () => {
  let ready = false;
  let sent = 0;
  const action = waitForSessionAction({ label: "Clear", ready: () => ready,
    reconnect: () => {}, unchanged: () => true, execute: async () => { sent++; } });
  sessionActionSnapshot()?.cancel();
  ready = true;
  await assertRejects(() => action, Error, "Action cancelled");
  assertEquals(sent, 0);
});

test("changed conversation and expired waits never dispatch", async () => {
  let ready = false;
  let sent = 0;
  const action = waitForSessionAction({ label: "Clear", ready: () => ready,
    reconnect: () => { ready = true; }, unchanged: () => false, execute: async () => { sent++; } });
  await assertRejects(() => action, Error, "conversation changed");
  await assertRejects(() => waitForSessionAction({ label: "Clear", ready: () => false,
    reconnect: () => {}, unchanged: () => true, timeoutMs: 0, execute: async () => { sent++; } }), Error, "Nothing was changed");
  assertEquals(sent, 0);
  assertEquals(sessionActionSnapshot(), null);
});

test("uncertain delivery is not replayed", async () => {
  let ready = false;
  let sent = 0;
  await assertRejects(() => waitForSessionAction({ label: "Clear", ready: () => ready,
    reconnect: () => { ready = true; }, unchanged: () => true,
    execute: async () => { sent++; throw new Error("ack lost"); } }), Error, "ack lost");
  assertEquals(sent, 1);
  assertEquals(sessionActionSnapshot(), null);
});

test("freshness is checked after hydration, not just socket readiness", async () => {
  let finish!: () => void;
  const hydration = new Promise<void>((resolve) => { finish = resolve; });
  let changed = false;
  let sent = 0;
  const action = waitForSessionAction({ label: "Clear", ready: () => true,
    reconnect: () => {}, prepare: () => hydration, unchanged: () => !changed,
    execute: async () => { sent++; } });
  assertEquals(sent, 0);
  changed = true;
  finish();
  await assertRejects(() => action, Error, "conversation changed");
  assertEquals(sent, 0);
});

test("cancel during hydration prevents sending after hydration finishes", async () => {
  let finish!: () => void;
  const hydration = new Promise<void>((resolve) => { finish = resolve; });
  let sent = 0;
  const action = waitForSessionAction({ label: "Clear", ready: () => true,
    reconnect: () => {}, prepare: () => hydration, unchanged: () => true,
    execute: async () => { sent++; } });
  sessionActionSnapshot()?.cancel();
  finish();
  await assertRejects(() => action, Error, "Action cancelled");
  assertEquals(sent, 0);
});
