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
