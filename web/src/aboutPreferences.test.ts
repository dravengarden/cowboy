import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { DEFAULT_ABOUT_VALUES, orderedItems, topBarUsageGroups, topBarUsageItems, usageTopBarGroupKey, withAboutDefaults } from "./aboutPreferences";

test("layout order survives renamed/offline items and appends newly available items", () => {
  const items = [{ id: "hawk", name: "Renamed", online: false }, { id: "ovh", name: "OVH", online: true }, { id: "new", name: "New", online: true }];
  assertEquals(orderedItems(items, ["gone", "ovh", "hawk"], (item) => item.id).map((item) => item.id), ["ovh", "hawk", "new"]);
  assertEquals(orderedItems(items, [], (item) => item.id), items);
  assertEquals(items.map((item) => item.id), ["hawk", "ovh", "new"]);
});

test("top-bar usage follows the Usage order and leaves out hidden accounts", () => {
  const items = [{ id: "husky" }, { id: "lark" }, { id: "claude" }, { id: "codex" }, { id: "new" }];
  const values = { ...DEFAULT_ABOUT_VALUES, usage_order: ["codex", "husky"], usage_top_bar_hidden: ["lark", "gone"] };
  assertEquals(topBarUsageItems(items, values, (item) => item.id).map((item) => item.id), ["codex", "husky", "claude", "new"]);
  assertEquals(topBarUsageItems(items, DEFAULT_ABOUT_VALUES, (item) => item.id), items);
});

test("a stored about value from before the top-bar field reads with its default", () => {
  const stored = { scope: "about" as const, revision: 3, values: { usage_order: ["a"], machine_order: [] } as never };
  assertEquals(withAboutDefaults(stored).values, { usage_order: ["a"], machine_order: [], usage_top_bar_hidden: [] });
});

test("a hidden quota group leaves only that group of its account out of the top bar", () => {
  const groups = [{ id: "gemini" }, { id: "claude-gpt" }, { id: "gemini-4-argon" }];
  const values = { ...DEFAULT_ABOUT_VALUES, usage_top_bar_hidden: [usageTopBarGroupKey("husky", "gemini-4-argon"), usageTopBarGroupKey("other", "gemini")] };
  assertEquals(usageTopBarGroupKey("husky", "gemini-4-argon"), "husky::gemini-4-argon");
  assertEquals(topBarUsageGroups("husky", groups, values).map((group) => group.id), ["gemini", "claude-gpt"]);
  assertEquals(topBarUsageGroups("husky", groups, DEFAULT_ABOUT_VALUES), groups);
});
