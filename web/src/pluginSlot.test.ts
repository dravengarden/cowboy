import { assert, assertEquals } from "jsr:@std/assert";

const passwordHost = await Deno.readTextFile(
  new URL("../../examples/authentication/password/host.json", import.meta.url),
);
const login = await Deno.readTextFile(
  new URL("./auth/ProductLoginPage.tsx", import.meta.url),
);

Deno.test("only external login methods mount through an isolated plugin slot", () => {
  assert(login.includes('from "@cowboy/plugin-api"'));
  assert(login.includes('slot="login.method"'));
  assert(login.includes("<PluginSlot"));
  assert(login.includes("pluginId={selectedProvider.id}"));
  assert(login.includes('loginContext?.kind === "password"'));
  assert(login.includes("context={loginContext}"));
  assert(login.includes("placeholder={null}"));
  assert(login.includes("LoginMethodFallback"));
  assert(login.includes("loginMethodLabel"));
  assert(login.includes("hostPlugins"));
  assert(login.includes("fieldLabels"));
  assert(login.includes("passwordLoginFields"));
  assertEquals(login.includes('label="Password"'), false);
  assertEquals(login.includes('return "Password"'), false);
  assertEquals(login.includes("PasswordStrength"), false);
  assertEquals(login.includes("passwordrules"), false);
  assert(login.includes('kind: "password"'));
  assert(login.includes('kind: "oidc"'));
  assert(login.includes("onSubmit: submit"));
  assert(login.includes("onAuthed"));
  assert(login.includes("onStart: submitProvider"));
  assert(login.includes("authApi.request"));
  assertEquals(login.includes("getCowboyPluginHost"), false);
  assert(passwordHost.includes('"label": "Password"'));
  assert(passwordHost.includes('"login.method": "login-password-v1"'));
  assertEquals(passwordHost.includes("ui/index.js"), false);
});

const providerManagement = await Deno.readTextFile(
  new URL("./ProviderManagement.tsx", import.meta.url),
);

Deno.test("provider lifecycle surfaces mount through plugin slots", () => {
  assert(providerManagement.includes('from "@cowboy/plugin-api"'));
  assert(providerManagement.includes("slot={kind}"));
  assert(providerManagement.includes("context={{"));
  assert(providerManagement.includes("kind,"));
  assert(providerManagement.includes("onEffect"));
  assert(providerManagement.includes("providerId={entry.provider_id}"));
  assert(providerManagement.includes('slot="provider.card"'));
  assert(providerManagement.includes("ProviderManagementCard"));
});

const infoSheet = await Deno.readTextFile(
  new URL("./InfoSheet.tsx", import.meta.url),
);
const desktopUsage = await Deno.readTextFile(
  new URL("./desktop/DesktopTopBarControls.tsx", import.meta.url),
);
const sessionSettings = await Deno.readTextFile(
  new URL("./sessionSettingsPresentation.ts", import.meta.url),
);
const usageWidget = await Deno.readTextFile(
  new URL("./usageWidget.ts", import.meta.url),
);
const usageLimitsSource = await Deno.readTextFile(
  new URL("./usageLimits.ts", import.meta.url),
);
const claudeCodeHost = await Deno.readTextFile(
  new URL("../../plugins/claude-code/host.json", import.meta.url),
);
const machineState = await Deno.readTextFile(
  new URL("./machineState.ts", import.meta.url),
);

Deno.test("provider usage cards mount through plugin slots", () => {
  assert(infoSheet.includes('slot="provider.usage"'));
  assert(infoSheet.includes("usagePluginId(usage.provider)"));
  assert(infoSheet.includes("context={usageContext}"));
  assert(infoSheet.includes("providerUsageSlotContext"));
  assert(infoSheet.includes("showDetails: true"));
  assertEquals(infoSheet.includes("<DeepSeekDetails"), false);
  assert(infoSheet.includes("usageAvailableStatus(usage.provider)"));
  assert(infoSheet.includes("usageOmitEmptyLimits(usage.provider)"));
  assertEquals(infoSheet.includes("deepseek-balance"), false);
  assertEquals(infoSheet.includes("/api/usage/deepseek/activity"), false);
  assert(desktopUsage.includes('slot="provider.usage"'));
  assert(desktopUsage.includes("usagePluginId(usage.provider)"));
  assert(desktopUsage.includes("context={usageContext}"));
  assert(desktopUsage.includes("usageWidgetHasBalance"));
  assertEquals(desktopUsage.includes('kind === "deepseek"'), false);
  assert(sessionSettings.includes("usageWidgetHasBalance"));
  assert(sessionSettings.includes("usageWidgetBalanceLabel"));
  assert(sessionSettings.includes("widget.kind"));
  assertEquals(sessionSettings.includes('id: "deepseek-balance"'), false);
  assertEquals(sessionSettings.includes('id: "deepseek-spend"'), false);
  assertEquals(sessionSettings.includes('kind === "deepseek"'), false);
  assert(usageWidget.includes("kind: usageWidgetKind(usage.provider)"));
  assert(usageWidget.includes("usageWidgetShape"));
  assert(usageWidget.includes("usageWidgetHasBalance"));
  assertEquals(usageWidget.includes('kind === "openai-weekly"'), false);
  assertEquals(usageWidget.includes('kind === "xai-included"'), false);
  assertEquals(usageWidget.includes('kind === "deepseek-balance"'), false);
  assertEquals(usageWidget.includes('kind: "deepseek"'), false);
  assertEquals(usageWidget.includes('kind: "openai"'), false);
  assertEquals(usageWidget.includes('kind: "xai"'), false);
  assert(usageLimitsSource.includes("usageLimitRowId"));
  assert(usageLimitsSource.includes("usageLimitLabel"));
  assertEquals(usageLimitsSource.includes('five_hour: { label: "5h"'), false);
  assertEquals(usageLimitsSource.includes("`claude-${kind}`"), false);
  assertEquals(
    usageLimitsSource.includes('if (parser === "xai-credits")'),
    false,
  );
  assertEquals(
    usageLimitsSource.includes('if (parser === "anthropic-utilization"'),
    false,
  );
  assertEquals(usageLimitsSource.includes("USAGE_LIMIT_PARSERS"), false);
  assertEquals(usageLimitsSource.includes("USAGE_ERROR_PRESENTERS"), false);
  assertEquals(
    usageLimitsSource.includes('if (kind === "openai-auth")'),
    false,
  );
  assertEquals(usageWidget.includes("function deepseekBalanceCny"), false);
  assertEquals(usageWidget.includes("function deepseekSpend24h"), false);
  assert(usageWidget.includes("activityCacheStats"));
  assert(usageWidget.includes("activityCostStats"));
  assert(usageWidget.includes('from "./activityUsage"'));
  assertEquals(usageWidget.includes("deepseekUsage"), false);
  assert(claudeCodeHost.includes('"limit_id_prefix": "claude"'));
  assert(claudeCodeHost.includes('"id": "five_hour"'));
  assert(claudeCodeHost.includes('"adapter_slot": "claude"'));
  assert(machineState.includes("providerOccupancySlot"));
  assert(machineState.includes("session.provider_generation_digest"));
  assertEquals(machineState.includes('slot === "claude"'), false);
  const diagnosticLogs = Deno.readTextFileSync(
    new URL("./diagnosticLogs.ts", import.meta.url),
  );
  assertEquals(diagnosticLogs.includes('["codex", "claude"]'), false);
  const usageLogs = Deno.readTextFileSync(
    new URL("./UsageLogs.tsx", import.meta.url),
  );
  assertEquals(usageLogs.includes('id: "codex", label: "Codex"'), false);
  assert(
    !new RegExp(
      "<PluginSlot[\\s\\S]*DesktopUsageExtras[\\s\\S]*<\\/PluginSlot>",
    )
      .test(desktopUsage),
  );
  assert(
    !infoSheet.includes("<PluginSlot") || !new RegExp(
      "<PluginSlot[\\s\\S]*ConfirmSheet[\\s\\S]*<\\/PluginSlot>",
    ).test(infoSheet),
  );
});

const pluginHost = await Deno.readTextFile(
  new URL("./pluginHost.ts", import.meta.url),
);
const pluginApi = await Deno.readTextFile(
  new URL("../../components/plugin-api/types.ts", import.meta.url),
);
const appleNativeBridge = await Deno.readTextFile(
  new URL(
    "../../apps/native-shell/apple/Sources/cowboy-app/CowboyPasskeyBridge.mm",
    import.meta.url,
  ),
);

Deno.test("host kit exposes only closed Cowboy-owned renderers", () => {
  assert(pluginHost.includes("ProviderUsage"));
  assert(pluginHost.includes("ProviderSurface"));
  assertEquals(pluginHost.includes("ProductPasskeysPanel"), false);
  assert(pluginHost.includes("RetiredLocalAuthenticationRenderer"));
  assert(pluginHost.includes("LoginMethodFallback"));
  assert(pluginHost.includes('from "./pluginUsage"'));
  assertEquals(pluginHost.includes("DeepSeekDetails"), false);
  assertEquals(pluginHost.includes("plugins/claude-deepseek"), false);
  assert(pluginApi.includes("host.ui.renderers[slot]"));
  assert(pluginApi.includes("installPluginRuntimeHosts"));
  assert(pluginApi.includes("__COWBOY_NATIVE_PLUGIN_HOST"));
  assert(pluginApi.includes("supportsNativePluginCapability"));
  assertEquals(pluginApi.includes("__COWBOY_PLUGIN_HOST"), false);
  assertEquals(pluginApi.includes("@vite-ignore"), false);
  assertEquals(pluginApi.includes("import("), false);
  assertEquals(pluginHost.includes("/api/plugins/"), false);
  assert(appleNativeBridge.includes("__COWBOY_NATIVE_PLUGIN_HOST"));
  assert(
    appleNativeBridge.includes("capabilities:Object.freeze(['webauthn'])"),
  );
});

for (const id of ["grok", "codex", "claude-code", "gemini"]) {
  const host = await Deno.readTextFile(
    new URL(`../../plugins/${id}/host.json`, import.meta.url),
  );
  Deno.test(`${id} selects closed provider renderers with data`, () => {
    assert(host.includes('"provider.usage": "provider-usage-v1"'));
    assert(host.includes('"provider.setup": "provider-surface-v1"'));
    assert(host.includes('"provider.settings": "provider-surface-v1"'));
    assertEquals(host.includes("ui/index.js"), false);
  });
}

const claudeDeepseekHost = await Deno.readTextFile(
  new URL("../../plugins/claude-deepseek/host.json", import.meta.url),
);
Deno.test("activity usage is a closed core renderer selected by host data", () => {
  assert(
    claudeDeepseekHost.includes(
      '"provider.usage": "provider-usage-activity-v1"',
    ),
  );
  assert(pluginHost.includes("ProviderUsageActivityRenderer"));
  assert(pluginHost.includes("ProviderUsageActivity"));
});

const passkeyHost = await Deno.readTextFile(
  new URL("../../examples/authentication/passkey/host.json", import.meta.url),
);
Deno.test("retained passkey packages stay readable without owning core account UI", () => {
  assert(passkeyHost.includes('"account.panel": "account-passkeys-v1"'));
  assertEquals(pluginHost.includes("ProductPasskeysPanel"), false);
  assert(
    pluginHost.includes(
      '"account-passkeys-v1": RetiredLocalAuthenticationRenderer',
    ),
  );
  assertEquals(passkeyHost.includes("ui/index.js"), false);
});

for (const id of ["google", "apple", "cloudflare-email"]) {
  const host = await Deno.readTextFile(
    new URL(`../../examples/authentication/${id}/host.json`, import.meta.url),
  );
  Deno.test(`${id} login method selects the closed OIDC renderer`, () => {
    assert(host.includes('"login.method": "login-oidc-v1"'));
    assertEquals(host.includes("ui/index.js"), false);
  });
}
