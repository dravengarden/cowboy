import { expect, test } from "bun:test";
import { createSocketGuards } from "./socketGuards.ts";

function harness() {
  let now = 0;
  let next = 1;
  const timers = new Map<number, { at: number; callback: () => void }>();
  const fired: string[] = [];
  const guards = createSocketGuards({
    onUpgradeTimeout: () => fired.push(`upgrade@${now}`),
    onBootstrapTimeout: () => fired.push(`bootstrap@${now}`),
    onSessionsTimeout: () => fired.push(`sessions@${now}`),
  }, {
    setTimeout: (callback, delayMs) => {
      const id = next++;
      timers.set(id, { at: now + delayMs, callback });
      return id;
    },
    clearTimeout: (handle) => {
      timers.delete(handle as number);
    },
  });
  const advance = (ms: number): void => {
    const end = now + ms;
    for (;;) {
      const due = [...timers].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      timers.delete(due[0]);
      now = due[1].at;
      due[1].callback();
    }
    now = end;
  };
  return { guards, advance, fired, timers };
}

test("a socket that never upgrades times out after 8s", () => {
  const { advance, fired } = harness();
  advance(7_999);
  expect(fired).toEqual([]);
  advance(1);
  expect(fired).toEqual(["upgrade@8000"]);
});

test("an opened socket must bootstrap within 12s of its last progress", () => {
  const { guards, advance, fired } = harness();
  advance(1_000);
  guards.upgraded();
  guards.awaitBootstrap();
  advance(10_000);
  guards.awaitBootstrap();
  advance(11_999);
  expect(fired).toEqual([]);
  advance(1);
  expect(fired).toEqual(["bootstrap@23000"]);
});

test("capacity waiting suspends the bootstrap deadline until progress resumes", () => {
  const { guards, advance, fired } = harness();
  guards.upgraded();
  guards.awaitBootstrap();
  guards.waitForCapacity();
  advance(60_000);
  expect(fired).toEqual([]);
  guards.awaitBootstrap();
  advance(12_000);
  expect(fired).toEqual(["bootstrap@72000"]);
});

test("admission requires the session list within 6s unless it is loaded", () => {
  const loading = harness();
  loading.guards.upgraded();
  loading.guards.awaitBootstrap();
  loading.guards.admitted(false);
  loading.advance(30_000);
  expect(loading.fired).toEqual(["sessions@6000"]);

  const loaded = harness();
  loaded.guards.upgraded();
  loaded.guards.admitted(true);
  loaded.advance(30_000);
  expect(loaded.fired).toEqual([]);
  expect(loaded.timers.size).toBe(0);
});

test("closing clears every pending deadline", () => {
  const { guards, advance, fired, timers } = harness();
  guards.closed();
  advance(60_000);
  expect(fired).toEqual([]);
  expect(timers.size).toBe(0);
});
