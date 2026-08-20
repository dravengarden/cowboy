import { assert, assertEquals } from "jsr:@std/assert";
import {
  appSettingsFromEvent,
  OPEN_APP_SETTINGS_EVENT,
} from "./appSettings.ts";

const reviewAppSource = await Deno.readTextFile(
  new URL("./mobile/review/ReviewApp.tsx", import.meta.url),
);
const reviewSettingsSource = await Deno.readTextFile(
  new URL("./mobile/review/ReviewSettings.tsx", import.meta.url),
);
const fileTreeSource = await Deno.readTextFile(
  new URL("./mobile/review/ReviewFileTree.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("Code settings open on the Code section and Agent settings stay on Agent", () => {
  const event = new CustomEvent(OPEN_APP_SETTINGS_EVENT, {
    detail: { section: "code" },
  });
  assertEquals(appSettingsFromEvent(event).section, "code");
  assertEquals(
    appSettingsFromEvent(new CustomEvent(OPEN_APP_SETTINGS_EVENT)).section,
    undefined,
  );
  assert(appSource.includes('label: "Agent"'));
  assert(appSource.includes('label: "Code"'));
  assert(appSource.includes("<ReviewSettingsContent"));
  assert(appSource.includes('openAppSettings({ section: "agent" })'));
  assert(appSource.includes("portal"));
});

Deno.test("Code chrome uses Agent instead of a local settings sheet", () => {
  assert(reviewAppSource.includes('data-mobile-open-agent="true"'));
  assert(reviewAppSource.includes('aria-label="Open Agent"'));
  assert(reviewAppSource.includes("openMobileProduct(\"agent\")"));
  assert(reviewAppSource.includes("data-mobile-review-mode-switcher"));
  assert(reviewAppSource.includes("data-mobile-review-mode-switch-track"));
  assert(reviewAppSource.includes("<Switch"));
  assert(reviewAppSource.includes('checked={mode === "files"}'));
  assert(
    reviewAppSource.includes(
      '"aria-label": "Switch between Git changes and Worktree files"',
    ),
  );
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
    reviewAppSource.includes('pr: "max(env(safe-area-inset-right, 0px), 10px)"'),
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
  assert(reviewSettingsSource.includes("export function ReviewSettingsContent"));
  assert(fileTreeSource.includes('openAppSettings({ section: "code" })'));
});
