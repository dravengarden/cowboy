import { assertEquals } from "jsr:@std/assert";
import {
  adjacentControlCenterTab,
  CONTROL_CENTER_TABS,
  controlCenterTabForShortcut,
} from "./controlCenterTabs.ts";

const appSource = await Deno.readTextFile(
  new URL("../App.tsx", import.meta.url),
);

Deno.test("control center numeric shortcuts select one stable tab", () => {
  assertEquals(
    CONTROL_CENTER_TABS.map(({ value, shortcut }) => [shortcut, value]),
    [
      ["1", "settings"],
      ["2", "machines"],
      ["3", "info"],
      ["4", "logs"],
    ],
  );
  assertEquals(controlCenterTabForShortcut("1"), "settings");
  assertEquals(controlCenterTabForShortcut("4"), "logs");
  assertEquals(controlCenterTabForShortcut("x"), null);
});

Deno.test("control center bracket navigation wraps across tabs", () => {
  assertEquals(adjacentControlCenterTab("settings", -1), "logs");
  assertEquals(adjacentControlCenterTab("settings", 1), "machines");
  assertEquals(adjacentControlCenterTab("logs", 1), "settings");
});

Deno.test("desktop control center keeps one stable semantic tab panel", () => {
  assertEquals(appSource.includes("<Tabs"), true);
  assertEquals(appSource.includes("selectionFollowsFocus"), true);
  assertEquals(
    appSource.includes('aria-label="Control center sections"'),
    true,
  );
  assertEquals(appSource.includes("aria-keyshortcuts={shortcut}"), true);
  assertEquals(appSource.includes('role="tabpanel"'), true);
  assertEquals(appSource.includes("controlCenterTabForShortcut(key)"), true);
  assertEquals(appSource.includes("adjacentControlCenterTab(tab"), true);
  assertEquals(appSource.includes("data-control-center-tab={tab}"), true);
  assertEquals(
    appSource.includes("data-control-center-panel-content"),
    true,
  );
  assertEquals(
    appSource.includes("data-control-center-rendered-tab={renderedTab}"),
    true,
  );
  assertEquals(appSource.includes("aria-busy={!tabPanelVisible}"), true);
  assertEquals(appSource.includes("startViewTransition.call("), true);
  assertEquals(
    appSource.includes("viewTransitionRef.current?.skipTransition()"),
    true,
  );
  assertEquals(appSource.includes("transition.ready.catch("), true);
  assertEquals(
    appSource.includes("transition.updateCallbackDone.catch("),
    true,
  );
  assertEquals(appSource.includes("flushSync(() =>"), true);
  assertEquals(appSource.includes("{tabContentReady && ("), false);
});

Deno.test("control center tab bar stays sticky on desktop and mobile scroll", () => {
  assertEquals(
    appSource.includes('position: desktop ? "sticky" : "static"'),
    false,
  );
  assertEquals(appSource.includes('position: "sticky"'), true);
  assertEquals(appSource.includes("top: desktop ? -1 : 0"), true);
  assertEquals(
    appSource.includes(
      'bgcolor: desktop ? "background.paper" : "background.default"',
    ),
    true,
  );
  assertEquals(appSource.includes("borderRadius: 0"), true);
  assertEquals(appSource.includes("borderBottom: desktop ? 0 : 1"), true);
  assertEquals(appSource.includes('borderColor: "divider"'), true);
});
