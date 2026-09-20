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
      ["2", "notifications"],
      ["3", "providers"],
      ["4", "machines"],
      ["5", "info"],
      ["6", "logs"],
      ["7", "account"],
    ],
  );
  assertEquals(controlCenterTabForShortcut("1"), "settings");
  assertEquals(controlCenterTabForShortcut("6"), "logs");
  assertEquals(controlCenterTabForShortcut("7"), "account");
  assertEquals(controlCenterTabForShortcut("x"), null);
});

Deno.test("control center bracket navigation wraps across tabs", () => {
  assertEquals(adjacentControlCenterTab("settings", -1), "account");
  assertEquals(adjacentControlCenterTab("settings", 1), "notifications");
  assertEquals(adjacentControlCenterTab("logs", 1), "account");
  assertEquals(adjacentControlCenterTab("account", 1), "settings");
  assertEquals(adjacentControlCenterTab("account", -1), "logs");
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

Deno.test("control center tab bar stays sticky on desktop", () => {
  assertEquals(
    appSource.includes('position: desktop ? "sticky" : "static"'),
    false,
  );
  assertEquals(appSource.includes('position: "sticky"'), true);
  assertEquals(appSource.includes("top: -1"), true);
  // The band still needs a fill so the sheet surface's scrolled content does
  // not bleed through it, but it is the frosted modal material rather than an
  // opaque paper fill, which printed a seam across the translucent dialog.
  assertEquals(
    appSource.includes(
      "bgcolor: (theme) => alpha(theme.palette.background.paper, 0.94)",
    ),
    true,
  );
  assertEquals(appSource.includes('backdropFilter: "blur(16px)"'), true);
  assertEquals(appSource.includes("borderRadius: 0"), true);
  assertEquals(appSource.includes("borderBottom: 0"), true);
  assertEquals(appSource.includes('borderColor: "divider"'), true);
});

/** Panel components rendered between two markers, in source order. */
function panelSequence(start: string, end: string): string[] {
  const from = appSource.indexOf(start);
  assertEquals(from >= 0, true, `missing marker: ${start}`);
  const to = appSource.indexOf(end, from + start.length);
  assertEquals(to >= 0, true, `missing terminator for: ${start}`);
  return [...appSource.slice(from, to).matchAll(/<(Product[A-Za-z]+)\b/g)]
    .map((match) => match[1]);
}

Deno.test("desktop Account tab renders the mobile account route's panels", () => {
  const desktop = panelSequence(
    "function DesktopAccountTabContent(): React.JSX.Element {",
    "\n}",
  );
  const mobile = panelSequence(
    "<Stack data-mobile-account-sections spacing={2}>",
    "</Stack>",
  );
  assertEquals(desktop, [
    "ProductSessionCapacityPanel",
    "ProductAccountSecurity",
    "ProductDevicesPanel",
    "ProductAccountMenu",
  ]);
  assertEquals(desktop, mobile);
});

Deno.test("desktop control center routes the account tab to that content", () => {
  assertEquals(
    appSource.includes(
      'renderedTab === "account" ? <DesktopAccountTabContent />',
    ),
    true,
  );
});
