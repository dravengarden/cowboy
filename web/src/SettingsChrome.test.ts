import { assert, assertEquals } from "jsr:@std/assert";

const chromeSource = await Deno.readTextFile(
  new URL("./SettingsChrome.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("mobile Settings is a preference list with drill-in destinations", () => {
  assert(chromeSource.includes("SettingsNavRow"));
  assert(chromeSource.includes("SETTINGS_MORE_ROWS"));
  assert(chromeSource.includes("SETTINGS_PROVIDER_ROW"));
  assert(chromeSource.includes('label: "About"'));
  assert(chromeSource.includes('label: "Accounts & sign-in"'));
  assert(appSource.includes("SETTINGS_PROVIDER_ROW"));
  assert(appSource.includes("<ProvidersContent"));
  assert(appSource.includes("key: \"info\""));
  assertEquals(chromeSource.includes("SettingsDestinationRail"), false);
  assertEquals(chromeSource.includes("SettingsProductSwitch"), false);
  assert(appSource.includes("<SettingsNavRow"));
  assert(appSource.includes('data-settings-section="code"'));
  assert(appSource.includes('label: "Back to Settings"'));
  assertEquals(appSource.includes('aria-label="Back to Settings"'), false);
  assert(appSource.includes('fontSize: "1.25em"'));
  assertEquals(appSource.includes("fontSize: 18, ml:"), false);
  const backIcon = appSource.slice(
    appSource.indexOf('label: "Back to Settings"'),
    appSource.indexOf('onPress: (): void => changeTab("settings")'),
  );
  assert(backIcon.includes("var(--cowboy-font-scale, 1)"));
  assertEquals(backIcon.includes('fontSize: "1.25em"'), false);
  assertEquals(backIcon.includes("fontSize: 18"), false);
  assert(appSource.includes('mobileDismiss={tab === "settings" || !useSheetSurface ? "footer" : "none"}'));
  assertEquals(appSource.includes("<SettingsDestinationRail"), false);
  assertEquals(appSource.includes("<SettingsProductSwitch"), false);
  assert(appSource.includes("portal"));
  assert(appSource.includes("borderRadius: 0"));
  assert(appSource.includes("(desktop || tab !== \"settings\") && ("));
  assertEquals(appSource.includes("<ThemeModeControl"), false);
  assertEquals(appSource.includes('sx={{ width: "100%", minHeight: 40 }}'), false);
  assert(appSource.includes('variant="subtitle1" fontWeight={740}'));
  assert(appSource.includes('data-settings-list="true"'));
  assert(appSource.includes("nextSavedSettingsScroll("));
  assert(appSource.includes("destinationScrollTop("));
});
