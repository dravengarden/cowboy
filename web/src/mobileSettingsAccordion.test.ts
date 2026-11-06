import { test } from "bun:test";
import { assert, assertEquals } from "../../tools/test-assert.ts";
import { readAppSource } from "./appSource.test.ts";

const appSource = await readAppSource();
test("mobile Settings uses an index and one lightweight detail route", () => {
  assert(appSource.includes("function MobileSettingsRoute"));
  assert(appSource.includes("initialMobileSettingsSection"));
  assert(appSource.includes('return focus === "code" ? "code" : null'));
  assert(appSource.includes("activeSection: MobileSettingsSection | null"));
  assert(appSource.includes("data-mobile-settings-route={id}"));
  assertEquals(appSource.includes("data-mobile-settings-detail-header"), false);
  assert(appSource.includes("<MobileSettingsNavigation"));
  assert(appSource.includes("hideMobileTitle={!desktop}"));
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
  assert(appSource.includes("Back to Settings from"));
  assert(appSource.includes('label: "Close settings"'));
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
  assert(/<ProvidersContent\s+embedded/u.test(appSource));
  assert(appSource.includes("<MachinesContent embedded"));
  assert(appSource.includes("<UsageLogs dense"));
  assert(appSource.includes("ProductDevicesPanel"));
  assert(appSource.includes("ProductSessionCapacityPanel"));
  assertEquals(appSource.includes("ProductTokensPanel"), false);
  assert(appSource.includes("<ProductAccountSecurity />"));
  assertEquals(appSource.includes('pluginId="passkey"'), false);
  assert(appSource.includes("ProductAccountMenu"));
  assert(appSource.includes('id="account"'));
  assert(appSource.includes('data-settings-section="code"'));
});

test("mobile Account keeps its independent cards visibly separated", () => {
  const routeStart = appSource.indexOf(
    '<MobileSettingsRoute\n                    id="account"',
  );
  const routeEnd = appSource.indexOf("</MobileSettingsRoute>", routeStart);
  assert(routeStart >= 0);
  assert(routeEnd > routeStart);
  const accountRoute = appSource.slice(routeStart, routeEnd);
  assert(
    /<Stack data-mobile-account-sections spacing=\{2\}>[\s\S]*<ProductSessionCapacityPanel \/>[\s\S]*<ProductAccountSecurity \/>[\s\S]*<ProductDevicesPanel hideWhenEmpty \/>[\s\S]*<ProductAccountMenu \/>[\s\S]*<\/Stack>/u
      .test(accountRoute),
  );
});
