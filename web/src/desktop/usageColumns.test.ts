import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { usageGroupRows } from "./UsageGroupedSegment";
import { USAGE_GROUP_ATTENTION_WIDTH_PX, USAGE_GROUP_WINDOW_WIDTH_PX, usageGroupSegmentWidth } from "./topBarDensity";

test("quota groups read as rows of labelled windows", () => {
  const now = 1_000_000_000_000;
  const window = (remaining: number, periodLabel: string, hours: number) => ({
    remaining,
    periodLabel,
    resetsAt: now / 1000 + hours * 3600,
  });
  const rows = usageGroupRows([
    { id: "gemini", label: "Gemini", binding: window(8, "Weekly", 30), windows: [window(34, "5h", 2), window(8, "Weekly", 30)] },
    { id: "claude-gpt", label: "Claude/GPT", binding: window(99, "Weekly", 100), windows: [window(100, "5h", 4), window(99, "Weekly", 100)] },
  ], now);
  assertEquals(rows.map((row) => [row.label, row.cells.map((cell) => `${cell.period} ${cell.primary}`)]), [
    ["Gemini", ["5h 34%", "W 8%"]],
    ["Claude/GPT", ["5h 100%", "W 99%"]],
  ]);
  // Every window that has spent quota shows its countdown; a full one has
  // nothing to recover. Only a low window takes the attention tone.
  assertEquals(rows.flatMap((row) => row.cells.map((cell) => cell.countdown)), ["2h", "1d6h", undefined, "4d4h"]);
  assertEquals(rows.flatMap((row) => row.cells.map((cell) => cell.attention)), [false, true, false, false]);
  assertEquals(rows[0]?.detail.startsWith("Gemini: 5h 34% ("), true);
  // Groups stack as rows: the width follows the windows per group, not the group count.
  assertEquals(usageGroupSegmentWidth(2), usageGroupSegmentWidth(1) + USAGE_GROUP_WINDOW_WIDTH_PX + USAGE_GROUP_ATTENTION_WIDTH_PX);
});
