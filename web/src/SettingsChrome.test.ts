import { assert, assertEquals } from "jsr:@std/assert";

const chromeSource = await Deno.readTextFile(
  new URL("./SettingsChrome.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("mobile Settings chrome is a title, product switch, and icon rail", () => {
  assert(chromeSource.includes("SettingsDestinationRail"));
  assert(chromeSource.includes("SettingsProductSwitch"));
  assert(chromeSource.includes("settingsDestinationLabel"));
  assert(chromeSource.includes('aria-label="Settings destinations"'));
  assert(chromeSource.includes('aria-label="Settings product"'));
  assertEquals(chromeSource.includes("SegmentedPill"), false);
  assert(appSource.includes("<SettingsDestinationRail"));
  assert(appSource.includes("<SettingsProductSwitch"));
  assert(appSource.includes("settingsDestinationLabel(tab)"));
  assert(appSource.includes("portal"));
});
