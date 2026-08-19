import { assert, assertEquals } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("mobile Settings is one mutually exclusive accordion surface", () => {
  assert(appSource.includes("function MobileSettingsAccordion"));
  assert(appSource.includes("initialMobileSettingsSection"));
  assert(appSource.includes('expanded={mobileSettingsSection === "appearance"}'));
  assert(appSource.includes('expanded={mobileSettingsSection === "notifications"}'));
  assert(appSource.includes('expanded={mobileSettingsSection === "providers"}'));
  assert(appSource.includes('expanded={mobileSettingsSection === "machines"}'));
  assert(appSource.includes('expanded={mobileSettingsSection === "info"}'));
  assert(appSource.includes('expanded={mobileSettingsSection === "logs"}'));
  assert(appSource.includes("onChange={setMobileSettingsSection}"));
  assert(appSource.includes('position: expanded ? "sticky" : "relative"'));
  assert(appSource.includes("MOBILE_SETTINGS_ANCHOR_MS"));
  assert(appSource.includes("surface.scrollTop += correction"));
  assert(appSource.includes('surface.addEventListener("pointerdown", cancelForUser'));
  assert(appSource.includes('surface.addEventListener("touchstart", cancelForUser'));
  assert(appSource.includes('surface.addEventListener("wheel", cancelForUser'));
  assert(appSource.includes("unmountOnExit: true"));
  assert(appSource.includes("prefers-reduced-motion: reduce"));
  assert(appSource.includes('borderRadius: "0 !important"'));
  assert(appSource.includes('data-settings-list="true" spacing={0}'));
  assert(appSource.includes('const openingTab = desktop ? initialTab : "settings"'));
  assertEquals(appSource.includes("<SettingsNavRow"), false);
  assertEquals(appSource.includes('label: "Back to Settings"'), false);
  assert(appSource.includes("<NotificationSettingsContent embedded"));
  assert(appSource.includes("<ProvidersContent embedded"));
  assert(appSource.includes("<MachinesContent embedded"));
  assert(appSource.includes("<UsageLogs dense"));
  assert(appSource.includes('data-settings-section="code"'));
  assert(appSource.includes('mobileDismiss="footer"'));
  assert(appSource.includes('data-settings-list="true"'));
});
