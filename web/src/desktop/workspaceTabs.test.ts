import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  type DesktopTabs,
  desktopTabAfterClose,
  desktopTabStep,
  desktopTabsToClose,
  keepDesktopTab,
  moveDesktopTab,
  NO_DESKTOP_TABS,
  pruneDesktopTabs,
  reopenDesktopTab,
  toggleDesktopTabPin,
  withDesktopTab,
  withoutDesktopTabs,
} from "./workspaceTabs";

const keys = (state: DesktopTabs): string =>
  state.tabs.map((tab) =>
    `${tab.pinned ? "^" : ""}${tab.key}${tab.preview ? "?" : ""}`
  ).join(" ");

/** `a b c`: kept tabs opened one after another. */
function kept(...names: string[]): DesktopTabs {
  let state = NO_DESKTOP_TABS;
  let previous: string | null = null;
  for (const name of names) {
    state = keepDesktopTab(withDesktopTab(state, name, previous), name);
    previous = name;
  }
  return state;
}

test("browsing reuses one preview tab until it is kept", () => {
  let state = withDesktopTab(NO_DESKTOP_TABS, "a");
  assertEquals(keys(state), "a?");
  state = withDesktopTab(state, "b", "a");
  assertEquals(keys(state), "b?");
  state = keepDesktopTab(state, "b");
  state = withDesktopTab(state, "c", "b");
  assertEquals(keys(state), "b c?");
  // An item that already has a tab only activates it.
  assertEquals(withDesktopTab(state, "b", "c"), state);
});

test("a new tab opens right of the current one, never among pinned tabs", () => {
  let state = kept("a", "b", "c");
  state = withDesktopTab(state, "d", "a");
  assertEquals(keys(state), "a d? b c");
  state = toggleDesktopTabPin(kept("a", "b"), "b");
  assertEquals(keys(state), "^b a");
  assertEquals(keys(withDesktopTab(state, "c", "b")), "^b c? a");
});

test("pinning keeps a preview tab and groups pinned tabs first", () => {
  let state = withDesktopTab(kept("a", "b"), "c", "b");
  state = toggleDesktopTabPin(state, "c");
  assertEquals(keys(state), "^c a b");
  state = toggleDesktopTabPin(state, "b");
  assertEquals(keys(state), "^c ^b a");
  state = toggleDesktopTabPin(state, "c");
  assertEquals(keys(state), "^b c a");
});

test("bulk closes spare pinned tabs; closing one tab does not", () => {
  const state = toggleDesktopTabPin(kept("a", "b", "c", "d", "e"), "a");
  const { tabs } = state;
  assertEquals(desktopTabsToClose(tabs, "c", "others"), ["b", "d", "e"]);
  assertEquals(desktopTabsToClose(tabs, "c", "left"), ["b"]);
  assertEquals(desktopTabsToClose(tabs, "c", "right"), ["d", "e"]);
  assertEquals(desktopTabsToClose(tabs, "c", "all"), ["b", "c", "d", "e"]);
  assertEquals(desktopTabsToClose(tabs, "a", "one"), ["a"]);
  assertEquals(keys(withoutDesktopTabs(state, ["b", "d"])), "^a c e");
});

test("closing the current tab returns to the last visited one, else a neighbour", () => {
  const { tabs } = kept("a", "b", "c", "d");
  assertEquals(desktopTabAfterClose(tabs, "c", ["c"], ["c", "a", "d"]), "a");
  assertEquals(desktopTabAfterClose(tabs, "c", ["c"], []), "d");
  assertEquals(desktopTabAfterClose(tabs, "d", ["d"], []), "c");
  assertEquals(desktopTabAfterClose(tabs, "c", ["a"], []), "c");
  assertEquals(desktopTabAfterClose(tabs, "a", ["a", "b", "c", "d"], []), null);
});

test("reopen restores the last closed item that still exists", () => {
  let state = withoutDesktopTabs(kept("a", "b", "c"), ["b"]);
  state = withoutDesktopTabs(state, ["c"]);
  let reopened = reopenDesktopTab(state, (key) => key !== "c", "a");
  assertEquals(reopened.key, "b");
  assertEquals(keys(reopened.state), "a b");
  reopened = reopenDesktopTab(reopened.state, () => true);
  assertEquals(reopened.key, null);
  // The preview tab is not replaced by a reopened one.
  state = withDesktopTab(withoutDesktopTabs(kept("a", "b"), ["b"]), "p", "a");
  assertEquals(keys(reopenDesktopTab(state, () => true, "p").state), "a p? b");
});

test("moving stays inside the tab's group and keeps a preview tab", () => {
  let state = toggleDesktopTabPin(kept("a", "b", "c"), "a");
  assertEquals(keys(moveDesktopTab(state, "c", 0)), "^a c b");
  assertEquals(keys(moveDesktopTab(state, "a", 2)), "^a b c");
  state = withDesktopTab(state, "d", "c");
  assertEquals(keys(moveDesktopTab(state, "d", 1)), "^a d b c");
});

test("stepping wraps and pruning drops deleted items", () => {
  const state = kept("a", "b", "c");
  assertEquals(desktopTabStep(state.tabs, "c", 1), "a");
  assertEquals(desktopTabStep(state.tabs, "a", -1), "c");
  assertEquals(desktopTabStep(state.tabs, null, 1), "a");
  assertEquals(desktopTabStep([], "a", 1), null);
  assertEquals(keys(pruneDesktopTabs(state, (key) => key !== "b")), "a c");
  assertEquals(pruneDesktopTabs(state, () => true), state);
});
