import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { usageColumns } from "./DesktopTopBarControls";
import { usageGroupSegmentWidth } from "./topBarDensity";

test("a quota group shows each window, named over its first", () => {
  const window = (remaining: number, periodLabel: string) => ({ remaining, periodLabel });
  const columns = usageColumns({
    kind: "none",
    label: "Antigravity",
    ...window(33, "5h"),
    groups: [
      { id: "gemini", label: "Gemini", binding: window(33, "5h"), windows: [window(33, "5h"), window(71, "Weekly")] },
      { id: "claude-gpt", label: "Claude/GPT", binding: window(99, "Weekly"), windows: [window(100, "5h"), window(99, "Weekly")] },
    ],
  }, "antigravity", 0);
  assertEquals(columns.map((column) => [column.group, column.primary, column.secondary.split(" · ")[0], column.groupStart]), [
    ["Gemini", "33%", "5h", undefined],
    [undefined, "71%", "Weekly", undefined],
    ["Claude/GPT", "100%", "5h", true],
    [undefined, "99%", "Weekly", undefined],
  ]);
  assertEquals(columns.filter((column) => column.detail !== undefined).length, 2);
  // Two groups of two windows cost two group columns, two bare windows and a gap.
  assertEquals(usageGroupSegmentWidth(2, 4), usageGroupSegmentWidth(2) + 2 * 100);
});
