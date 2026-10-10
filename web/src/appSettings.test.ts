import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import {
  appSettingsFromEvent,
  OPEN_APP_SETTINGS_EVENT,
} from "./appSettings.ts";

const reviewAppSource = await readFile(
  new URL("./mobile/review/ReviewApp.tsx", import.meta.url), "utf8",
);
const reviewSettingsSource = await readFile(
  new URL("./mobile/review/ReviewSettings.tsx", import.meta.url), "utf8",
);
const fileTreeSource = await readFile(
  new URL("./mobile/review/ReviewFileTree.tsx", import.meta.url), "utf8",
);
const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);

test("Code settings open on the Code section and Agent settings stay on Agent", () => {
  const event = new CustomEvent(OPEN_APP_SETTINGS_EVENT, {
    detail: { section: "code" },
  });
  assertEquals(appSettingsFromEvent(event).section, "code");
  assertEquals(
    appSettingsFromEvent(new CustomEvent(OPEN_APP_SETTINGS_EVENT)).section,
    undefined,
  );
  assert(appSource.includes('label: "Agent"'));
  assert(appSource.includes('label: "Code & diff"'));
  assert(appSource.includes("<ReviewSettingsContent"));
  assert(appSource.includes('openAppSettings({ section: "agent" })'));
  assert(appSource.includes("portal"));
});

test("desktop settings use compact section tabs and expose product sign out", () => {
  const settings = appSource.slice(
    appSource.indexOf("function DesktopSettingsContent("),
    appSource.indexOf("function machineComponentName("),
  );
  assert(settings.includes("data-desktop-settings-section-tabs"));
  assert(settings.includes("fullWidth={false}"));
  assert(settings.includes('label: "Code & diff"'));
  // Account is its own control center tab rather than a Settings block, so
  // sign out sits beside Passkeys and client credentials exactly as it does
  // in the mobile account route. controlCenterTabs.test.ts pins that parity.
  assert(settings.includes("function DesktopAccountTabContent("));
  assert(settings.includes("<ProductAccountMenu />"));
  assert(settings.includes("<SegmentedTabs"));
});

test("Machine mutations use correlated Controller receipts instead of polling event history", () => {
  const machines = appSource.slice(
    appSource.indexOf("function MachinesContent("),
    appSource.indexOf("function isSettingsEditableTarget("),
  );
  assert(
    machines.includes(
      "await fetch(`/api/machines/${encodeURIComponent(machineId)}/refresh`",
    ),
  );
  assert(
    machines.includes(
      "await fetch(`/api/machines/${encodeURIComponent(machineId)}/components/update-npm`",
    ),
  );
  assertEquals(machines.includes("/events"), false);
  assertEquals(machines.includes("request_id"), false);
});

test("Code chrome uses Agent instead of a local settings sheet", () => {
  assert(reviewAppSource.includes('data-mobile-open-agent="true"'));
  assert(reviewAppSource.includes('aria-label="Open Agent"'));
  assert(reviewAppSource.includes('openMobileProduct("agent")'));
  assert(reviewAppSource.includes("data-mobile-review-mode-switcher"));
  assertEquals(
    reviewAppSource.includes("data-mobile-review-mode-switch-track"),
    false,
  );
  assertEquals(
    reviewAppSource.includes("<Switch"),
    false,
  );
  assert(
    reviewAppSource.includes(
      'aria-label="Switch between Git changes and Worktree files"',
    ),
  );
  {
    const switcher = reviewAppSource.slice(
      reviewAppSource.indexOf("function ReviewModeSwitcher("),
      reviewAppSource.indexOf("function sessionStatusColor("),
    );
    assertEquals(switcher.includes("translateX"), false);
    assertEquals(switcher.includes("boxShadow"), false);
    assert(switcher.includes('transform: "none"'));
    assert(switcher.includes("height: 40"));
    assert(switcher.includes('alignItems: "center"'));
    assert(switcher.includes('boxSizing: "border-box"'));
    assert(switcher.includes('fontSize: "1.125rem"'));
    assert(switcher.includes("theme.palette.primary.main"));
    assert(switcher.includes("disableRipple"));
    assert(switcher.includes("dataset.touchActivated"));
    assert(switcher.includes("&[aria-pressed='true']"));
    assert(switcher.includes("&&[aria-pressed='true']"));
    assert(
      switcher.includes(
        "[data-touch-activated='true'][aria-pressed='false']:hover",
      ),
    );
    assert(switcher.includes("COARSE_POINTER_ROOT_CLASS"));
    assert(
      switcher.includes(
        "&&[aria-pressed='true']:hover",
      ),
    );
    assertEquals(switcher.includes("fontSize: 17"), false);
    assertEquals(
      switcher.includes('bgcolor: mode === "git" ? "background.paper"'),
      false,
    );
  }
  assert(reviewAppSource.includes('data-mobile-review-sidebar="true"'));
  assert(
    reviewAppSource.indexOf('data-mobile-open-agent="true"') >
      reviewAppSource.indexOf('aria-label="Code Review controls"'),
  );
  {
    const header = reviewAppSource.slice(
      reviewAppSource.indexOf("minHeight: 52"),
      reviewAppSource.indexOf('aria-label="Code Review controls"'),
    );
    const controls = reviewAppSource.slice(
      reviewAppSource.indexOf('aria-label="Code Review controls"'),
      reviewAppSource.indexOf("data-review-tab-close-confirm"),
    );
    assert(header.includes("<ReviewModeSwitcher"));
    assertEquals(controls.includes("data-mobile-review-mode-switcher"), false);
    assertEquals(controls.includes("<ReviewModeSwitcher"), false);
  }
  assert(reviewAppSource.includes("ChatBubbleOutline"));
  assert(
    reviewAppSource.includes('pl: "env(safe-area-inset-left, 0px)"'),
  );
  assert(
    reviewAppSource.includes('pr: "env(safe-area-inset-right, 0px)"'),
  );
  assertEquals(
    reviewAppSource.includes(
      'pr: "max(env(safe-area-inset-right, 0px), 10px)"',
    ),
    false,
  );
  {
    const toolbar = reviewAppSource.slice(
      reviewAppSource.indexOf('aria-label="Code Review controls"'),
      reviewAppSource.indexOf('data-mobile-open-agent="true"'),
    );
    assert(toolbar.includes("px: 2"));
    assertEquals(toolbar.includes("px: 1"), false);
  }
  assertEquals(reviewAppSource.includes("ArrowBackIosNew"), false);
  assertEquals(reviewSettingsSource.includes("SettingsSheet"), false);
  assert(
    reviewSettingsSource.includes("export function ReviewSettingsContent"),
  );
  assert(fileTreeSource.includes('openAppSettings({ section: "code" })'));
});

test("Context tabs use the shared segmented tabs", () => {
  const tabs = reviewAppSource.slice(
    reviewAppSource.indexOf("data-mobile-context-tabs"),
    reviewAppSource.indexOf(
      'contextTab === "sessions"',
      reviewAppSource.indexOf("data-mobile-context-tabs"),
    ),
  );
  // SegmentedTabs.test.ts pins the iOS selected-pill invariants.
  assert(tabs.includes("Context views"));
  assert(
    reviewAppSource.includes(
      '<SegmentedTabs\n                rootProps={{ "data-mobile-context-tabs": "" }}',
    ),
  );
});
