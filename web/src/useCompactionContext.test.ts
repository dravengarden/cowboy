import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { contextValueFromSessionInfo, freshContextReading } from "./sessionInfoContext.ts";

test("session info context decoder reads flattened SessionMeta fields", () => {
  assertEquals(
    contextValueFromSessionInfo({
      id: "session-1",
      status: "running",
      context_used: 21_000,
      context_size: 353_400,
      event_count: 42,
    }),
    { used: 21_000, size: 353_400, observedAt: 0, stale: false },
  );
});

test("session info context decoder rejects the obsolete nested shape", () => {
  assertEquals(
    contextValueFromSessionInfo({
      meta: { context_used: 21_000, context_size: 353_400 },
    }),
    null,
  );
});

test("session info context decoder rejects incomplete values", () => {
  assertEquals(contextValueFromSessionInfo({ context_used: 21_000 }), null);
  assertEquals(
    contextValueFromSessionInfo({ context_used: -1, context_size: 353_400 }),
    null,
  );
});

test("session info context decoder reads the reading's identity and stale mark", () => {
  assertEquals(
    contextValueFromSessionInfo({
      context_used: 21_000,
      context_size: 353_400,
      usage: { used: 21_000, size: 353_400, observed_at_ms: 7, compacted_at_ms: 9 },
    }),
    { used: 21_000, size: 353_400, observedAt: 7, stale: true },
  );
});

test("a reading is fresh by its observation time, never when marked stale", () => {
  const baseline = { used: 100, size: 1_000, observedAt: 7, stale: false };
  // A fresh report repeating the old counters settles the refresh.
  assertEquals(freshContextReading({ ...baseline, observedAt: 8 }, baseline), true);
  // Historical counters that differ from the click-time baseline do not.
  assertEquals(freshContextReading({ ...baseline, used: 200 }, baseline), false);
  assertEquals(
    freshContextReading({ ...baseline, used: 200, observedAt: 8, stale: true }, baseline),
    false,
  );
  // Without observation times, the counters decide.
  const unknown = { ...baseline, observedAt: 0 };
  assertEquals(freshContextReading({ ...unknown, used: 200 }, unknown), true);
  assertEquals(freshContextReading(unknown, unknown), false);
});
