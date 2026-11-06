import { expect, test } from "bun:test";
import { createLivenessWatch, type LivenessClock } from "./livenessWatch.ts";

function manualClock() {
  let now = 10_000;
  const intervals = new Map<number, { every: number; next: number; callback: () => void }>();
  let id = 1;
  const clock: LivenessClock = {
    now: () => now,
    setInterval: (callback, every) => {
      intervals.set(id, { every, next: now + every, callback });
      return id++;
    },
    clearInterval: (handle) => {
      intervals.delete(handle as number);
    },
  };
  const advance = (ms: number): void => {
    const end = now + ms;
    for (;;) {
      const due = [...intervals.values()].filter((t) => t.next <= end)
        .sort((a, b) => a.next - b.next)[0];
      if (!due) break;
      now = due.next;
      due.next += due.every;
      due.callback();
    }
    now = end;
  };
  return { clock, advance, intervals };
}

const limits = { staleMs: 60_000, checkMs: 15_000 };

test("a silent open socket is stale after the limit, not before", () => {
  const { clock, advance } = manualClock();
  const watch = createLivenessWatch(limits, clock);
  let stale = 0;
  let checks = 0;
  watch.markAlive();
  watch.start({ isOpen: () => true, onStale: () => stale++, onCheck: () => checks++ });
  advance(60_000);
  expect([stale, checks]).toEqual([0, 4]);
  advance(15_000);
  expect(stale).toBe(1);
});

test("any frame resets the silence", () => {
  const { clock, advance } = manualClock();
  const watch = createLivenessWatch(limits, clock);
  let stale = 0;
  watch.markAlive();
  watch.start({ isOpen: () => true, onStale: () => stale++, onCheck: () => {} });
  for (let i = 0; i < 10; i++) {
    advance(25_000);
    watch.markAlive();
  }
  expect(stale).toBe(0);
  expect(watch.lastMessageAt).toBe(10_000 + 250_000);
});

test("a socket that is no longer current or open is never reported stale", () => {
  const { clock, advance } = manualClock();
  const watch = createLivenessWatch(limits, clock);
  let stale = 0;
  let checks = 0;
  watch.start({ isOpen: () => false, onStale: () => stale++, onCheck: () => checks++ });
  advance(300_000);
  expect(stale).toBe(0);
  expect(checks).toBe(20);
});

test("restarting replaces the previous check and stop ends it", () => {
  const { clock, advance, intervals } = manualClock();
  const watch = createLivenessWatch(limits, clock);
  let first = 0;
  let second = 0;
  watch.start({ isOpen: () => false, onStale: () => {}, onCheck: () => first++ });
  watch.start({ isOpen: () => false, onStale: () => {}, onCheck: () => second++ });
  expect(intervals.size).toBe(1);
  advance(15_000);
  expect([first, second]).toEqual([0, 1]);
  watch.stop();
  expect(intervals.size).toBe(0);
});
