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
  assert(reviewAppSource.includes('aria-label="Back to Agent"'));
  assert(reviewAppSource.includes("ArrowBackIosNew"));
  assert(reviewAppSource.includes("openMobileProduct(\"agent\")"));
  assertEquals(reviewAppSource.includes("ChatBubbleOutline"), false);
  assertEquals(reviewSettingsSource.includes("SettingsSheet"), false);
  assert(reviewSettingsSource.includes("export function ReviewSettingsContent"));
  assert(fileTreeSource.includes('openAppSettings({ section: "code" })'));
});
