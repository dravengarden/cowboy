import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert";
import { recordedContextNote, recordedContextUsageAt } from "./contextUsageRecord";
import {
  COMPACTED_CONTEXT_HINT,
  compactTooltip,
  contextStaleAfterCompaction,
} from "./contextUsageRecord";

const usage = { used: 780_000, size: 1_000_000, raw: null, observed_at_ms: 1_000 };

test("a dormant session's persisted reading is marked with its age", () => {
  assertEquals(recordedContextUsageAt({ status: "exited", usage }), 1_000);
  assertEquals(recordedContextUsageAt({ status: "starting", usage }), 1_000);
  assertEquals(recordedContextNote(1_000, 1_000 + 3 * 3_600_000), " · last recorded 3h ago");
});

test("a live or unreported reading carries no recorded note", () => {
  assertEquals(recordedContextUsageAt({ status: "running", usage }), undefined);
  assertEquals(recordedContextUsageAt({ status: "busy", usage }), undefined);
  assertEquals(recordedContextUsageAt({ status: "exited" }), undefined);
  assertEquals(
    recordedContextUsageAt({ status: "exited", usage: { ...usage, size: 0 } }),
    undefined,
  );
  assertEquals(recordedContextNote(undefined), "");
});

test("the Compact tooltip appends the recorded note only when given", () => {
  const live = compactTooltip({ used: 780_000, size: 1_000_000 });
  assertEquals(live.includes("last recorded"), false);
  assertEquals(
    compactTooltip({ used: 780_000, size: 1_000_000 }, Date.now()).endsWith(
      " · last recorded just now",
    ),
    true,
  );
  assertEquals(compactTooltip({ used: 0, size: 0 }, 1), "Compact conversation");
});

test("a reading a compaction left as history is stale until replaced", () => {
  assertEquals(contextStaleAfterCompaction(undefined), false);
  assertEquals(contextStaleAfterCompaction({ usage }), false);
  assertEquals(contextStaleAfterCompaction({ usage: { ...usage, compacted_at_ms: 2_000 } }), true);
  // The tooltip names the unknown state, never the old percentage.
  assertEquals(
    compactTooltip({ used: 780_000, size: 1_000_000, stale: true }, 1_000),
    `Compact conversation · ${COMPACTED_CONTEXT_HINT}`,
  );
  assertEquals(COMPACTED_CONTEXT_HINT, "Compacted · updates after the next reply");
});
