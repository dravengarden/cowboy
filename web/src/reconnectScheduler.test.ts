import { expect, test } from "bun:test";
import { createReconnectScheduler, type ReconnectClock } from "./reconnectScheduler.ts";

function manualClock() {
  let now = 1_000;
  const timers = new Map<number, { at: number; callback: () => void }>();
  let next = 1;
  const clock: ReconnectClock = {
    now: () => now,
    setTimeout: (callback, delayMs) => {
      const id = next++;
      timers.set(id, { at: now + delayMs, callback });
      return id;
    },
    clearTimeout: (handle) => {
      timers.delete(handle as number);
    },
  };
  const advance = (ms: number): void => {
    now += ms;
    for (const [id, timer] of [...timers]) {
      if (timer.at <= now) {
        timers.delete(id);
        timer.callback();
      }
    }
  };
  return { clock, advance, timers };
}

test("an attempt fires once at its delay and then clears its countdown", () => {
  const { clock, advance } = manualClock();
  const scheduler = createReconnectScheduler(clock);
  let fired = 0;
  expect(scheduler.schedule(2_000, () => fired++)).toBe(true);
  expect(scheduler.pending).toBe(true);
  expect(scheduler.retryAt).toBe(3_000);
  advance(1_999);
  expect(fired).toBe(0);
  advance(1);
  expect(fired).toBe(1);
  expect(scheduler.pending).toBe(false);
  expect(scheduler.retryAt).toBeUndefined();
});

test("only one attempt is pending at a time", () => {
  const { clock, advance, timers } = manualClock();
  const scheduler = createReconnectScheduler(clock);
  let first = 0;
  let second = 0;
  scheduler.schedule(1_000, () => first++);
  expect(scheduler.schedule(5_000, () => second++)).toBe(false);
  expect(timers.size).toBe(1);
  expect(scheduler.retryAt).toBe(2_000);
  advance(5_000);
  expect([first, second]).toEqual([1, 0]);
});

test("cancelling drops the attempt and its countdown, and allows a new one", () => {
  const { clock, advance, timers } = manualClock();
  const scheduler = createReconnectScheduler(clock);
  let fired = 0;
  scheduler.schedule(1_000, () => fired++);
  scheduler.cancel();
  expect(scheduler.pending).toBe(false);
  expect(scheduler.retryAt).toBeUndefined();
  expect(timers.size).toBe(0);
  advance(2_000);
  expect(fired).toBe(0);
  expect(scheduler.schedule(500, () => fired++)).toBe(true);
  advance(500);
  expect(fired).toBe(1);
});

test("cancelling with nothing pending is harmless", () => {
  const { clock } = manualClock();
  const scheduler = createReconnectScheduler(clock);
  scheduler.cancel();
  expect(scheduler.pending).toBe(false);
});
