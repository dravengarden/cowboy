import { assertEquals } from "jsr:@std/assert";
import {
  destinationScrollTop,
  nextSavedSettingsScroll,
  settingsFocusRevealDelta,
} from "./settingsDrillInScroll.ts";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("leaving Settings captures the list offset and other pages keep it", () => {
  assertEquals(nextSavedSettingsScroll("settings", 420, 0), 420);
  assertEquals(nextSavedSettingsScroll("providers", 12, 420), 420);
  assertEquals(nextSavedSettingsScroll("settings", -8, 10), 0);
});

Deno.test("returning to Settings restores the list; drill-in pages start at top", () => {
  assertEquals(destinationScrollTop("settings", 420), 420);
  assertEquals(destinationScrollTop("providers", 420), 0);
  assertEquals(destinationScrollTop("machines", 420), 0);
  assertEquals(destinationScrollTop("settings", -4), 0);
});

Deno.test("SettingsShell restores list scroll after a mobile drill-in back", () => {
  assertEquals(appSource.includes("nextSavedSettingsScroll("), true);
  assertEquals(appSource.includes("destinationScrollTop("), true);
  assertEquals(appSource.includes('data-settings-list="true"'), true);
});

Deno.test("focused settings stay between the sticky header and floating navigation", () => {
  const visible = { top: 100, bottom: 464 };
  assertEquals(
    settingsFocusRevealDelta({ top: 438, bottom: 472 }, visible),
    20,
  );
  assertEquals(
    settingsFocusRevealDelta({ top: 104, bottom: 138 }, visible),
    -8,
  );
  assertEquals(
    settingsFocusRevealDelta({ top: 180, bottom: 220 }, visible),
    0,
  );
});
