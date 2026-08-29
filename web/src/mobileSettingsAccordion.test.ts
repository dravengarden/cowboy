import { assert, assertEquals } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
Deno.test("mobile Settings uses an index and one lightweight detail route", () => {
  assert(appSource.includes("function MobileSettingsRoute"));
  assert(appSource.includes("initialMobileSettingsSection"));
  assert(appSource.includes('return focus === "code" ? "code" : null'));
  assert(appSource.includes("activeSection: MobileSettingsSection | null"));
  assert(appSource.includes("data-mobile-settings-route={id}"));
  assert(appSource.includes("data-mobile-settings-detail-header"));
  assert(appSource.includes('variant="overline"'));
  assert(appSource.includes('letterSpacing: "0.12em"'));
  assert(appSource.includes('<Typography variant="h6" fontWeight={780}'));
  assert(
    /data-mobile-settings-level=\{\s*mobileSettingsSection === null\s*\? "index"\s*: "detail"\s*\}/u
      .test(appSource),
  );
  assert(appSource.includes("changeMobileSettingsSection"));
  assert(appSource.includes("MOBILE_SETTINGS_HEAVY_CONTENT_DELAY_MS = 180"));
  assert(appSource.includes('id === "machines" || id === "providers"'));
  assert(appSource.includes("contentReady"));
  assert(appSource.includes("mobile-settings-content-enter 100ms"));
  assert(
    appSource.includes("settingsListScrollRef.current = surface.scrollTop"),
  );
  assert(appSource.includes("mobile-settings-route-enter 140ms"));
  assert(appSource.includes('willChange: "transform, opacity"'));
  assert(appSource.includes("prefers-reduced-motion: reduce"));
  assert(
    appSource.includes(
      'key: mobileSettingsSection === null ? "close" : "back"',
    ),
  );
  assert(
    appSource.includes(
      'label: mobileSettingsSection === null ? "Close" : "Back"',
    ),
  );
  assert(appSource.includes("<MobileSheetActionGroup"));
  assert(appSource.includes("data-mobile-settings-navigation"));
  assert(appSource.includes("KEYBOARD_INSET_CHANGED_EVENT"));
  assert(appSource.includes("settingsFocusRevealDelta("));
  assert(
    appSource.includes('viewport?.addEventListener("resize", scheduleReveal)'),
  );
  assert(appSource.includes("<ArrowBackIosNew"));
  assert(appSource.includes("changeMobileSettingsSection(null)"));
  assertEquals(appSource.includes("<Accordion"), false);
  assertEquals(appSource.includes("MOBILE_SETTINGS_ANCHOR"), false);
  assert(appSource.includes("<NotificationSettingsContent embedded"));
  assert(appSource.includes("<ProvidersContent embedded"));
  assert(appSource.includes("<MachinesContent embedded"));
  assert(appSource.includes("<UsageLogs dense"));
  assert(appSource.includes("ProductTokensPanel"));
  assert(appSource.includes("ProductPasskeysPanel"));
  assert(appSource.includes("ProductAccountMenu"));
  assert(appSource.includes('id="account"'));
  assert(appSource.includes('data-settings-section="code"'));
});
