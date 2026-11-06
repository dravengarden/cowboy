import { expect, test } from "bun:test";
import { createOutageTracker } from "./outageTracker.ts";

test("an outage keeps its first start and measures from it", () => {
  let now = 1_000;
  const outage = createOutageTracker(() => now);
  expect(outage.active).toBe(false);
  expect(outage.durationMs()).toBe(0);
  outage.begin();
  now = 4_000;
  outage.begin();
  expect(outage.active).toBe(true);
  expect(outage.durationMs()).toBe(3_000);
});

test("recovery ends the outage, clears its counters and marks the Hub live", () => {
  let now = 1_000;
  const outage = createOutageTracker(() => now);
  outage.begin();
  outage.countAttempt();
  outage.countAttempt();
  outage.countFailure();
  expect([outage.attempts, outage.failures]).toEqual([2, 1]);
  now = 9_000;
  outage.recovered();
  expect(outage.active).toBe(false);
  expect([outage.attempts, outage.failures]).toEqual([0, 0]);
  expect(outage.lastLiveAt).toBe(9_000);
});

test("a cached replica's live time never overrides a known one", () => {
  let now = 5_000;
  const outage = createOutageTracker(() => now);
  outage.restoreLastLive(2_000);
  expect(outage.lastLiveAt).toBe(2_000);
  outage.restoreLastLive(3_000);
  expect(outage.lastLiveAt).toBe(2_000);
  now = 7_000;
  outage.markLive();
  expect(outage.lastLiveAt).toBe(7_000);
});
