import { assert, assertEquals } from "jsr:@std/assert";

const passwordHost = await Deno.readTextFile(
  new URL("../../examples/authentication/password/host.json", import.meta.url),
);
const passwordUi = await Deno.readTextFile(
  new URL("../../examples/authentication/password/ui/index.js", import.meta.url),
);
const login = await Deno.readTextFile(
  new URL("./auth/ProductLoginPage.tsx", import.meta.url),
);

Deno.test("login methods mount through an isolated plugin slot", () => {
  assert(login.includes('from "@cowboy/plugin-api"'));
  assert(login.includes('slot="login.method"'));
  assert(login.includes("<PluginSlot"));
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
  assert(login.includes("auth.startOidc"));
  assert(login.includes("getCowboyPluginHost().auth"));
  assertEquals(login.includes("authApi.login"), false);
  assertEquals(login.includes("authApi.register"), false);
  assertEquals(login.includes("authApi.setup"), false);
  assert(passwordHost.includes('"label": "Password"'));
  assert(passwordUi.includes("submitPassword"));
  assert(passwordUi.includes("context.fieldLabels?.account"));
  assert(passwordUi.includes("context.fieldLabels?.secret"));
  assert(passwordUi.includes("host().auth"));
  assert(passwordUi.includes('type: "button"'));
  assert(passwordUi.includes("PasswordStrength"));
  assert(passwordUi.includes("passwordrules"));
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
  assert(providerManagement.includes('providerId={entry.provider_id}'));
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
  assertEquals(usageLimitsSource.includes('if (parser === "xai-credits")'), false);
  assertEquals(
    usageLimitsSource.includes('if (parser === "anthropic-utilization"'),
    false,
  );
  assert(usageLimitsSource.includes("USAGE_LIMIT_PARSERS"));
  assert(usageLimitsSource.includes("USAGE_ERROR_PRESENTERS"));
  assertEquals(usageLimitsSource.includes('if (kind === "openai-auth")'), false);
  assertEquals(usageWidget.includes("function deepseekBalanceCny"), false);
  assertEquals(usageWidget.includes("function deepseekSpend24h"), false);
  assert(usageWidget.includes("activityCacheStats"));
  assert(usageWidget.includes("activityCostStats"));
  assert(usageWidget.includes('from "./activityUsage"'));
  assertEquals(usageWidget.includes("deepseekUsage"), false);
  assert(claudeCodeHost.includes('"limit_id_prefix": "claude"'));
  assert(claudeCodeHost.includes('"id": "five_hour"'));
  assert(claudeCodeHost.includes('"adapter_slot": "claude"'));
  assert(machineState.includes("occupancyProviderIds"));
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
    !new RegExp("<PluginSlot[\\s\\S]*DesktopUsageExtras[\\s\\S]*<\\/PluginSlot>")
      .test(desktopUsage),
  );
  assert(!infoSheet.includes("<PluginSlot") || !new RegExp(
    "<PluginSlot[\\s\\S]*ConfirmSheet[\\s\\S]*<\\/PluginSlot>",
  ).test(infoSheet));
});

const sharedUsage = await Deno.readTextFile(
  new URL("../../plugins/provider-usage-slot.js", import.meta.url),
);
const pluginHost = await Deno.readTextFile(
  new URL("./pluginHost.ts", import.meta.url),
);

Deno.test("host kit installs shared provider and account components", () => {
  assert(pluginHost.includes("ProviderUsage"));
  assert(pluginHost.includes("DeepSeekDetails"));
  assert(pluginHost.includes("ProviderSurface"));
  assert(pluginHost.includes("PasskeysPanel"));
  assert(pluginHost.includes("authApi.login"));
  assert(pluginHost.includes("authApi.register"));
  assert(pluginHost.includes("authApi.setup"));
  assert(pluginHost.includes('from "./pluginUsage"'));
  assert(pluginHost.includes("/api/plugins/"));
  assert(pluginHost.includes("/call"));
  assert(pluginHost.includes("plugins/claude-deepseek/ui/DeepSeekDetails"));
});

for (const id of ["grok", "codex", "claude-code", "gemini"]) {
  const usage = await Deno.readTextFile(
    new URL(`../../plugins/${id}/ui/index.js`, import.meta.url),
  );
  Deno.test(`${id} usage module reuses the host ProviderUsage component`, () => {
    assertEquals(usage, sharedUsage);
    assert(usage.includes("components.ProviderUsage"));
    assert(usage.includes("components.ProviderSurface"));
    assert(usage.includes('"provider.usage": ProviderUsageSlot'));
    assert(usage.includes('"provider.setup": ProviderLifecycleSlot'));
    assert(usage.includes('"provider.settings": ProviderLifecycleSlot'));
  });
}

const claudeDeepseekUsage = await Deno.readTextFile(
  new URL("../../plugins/claude-deepseek/ui/index.js", import.meta.url),
);
Deno.test("claude-deepseek usage module mounts host DeepSeek details", () => {
  assertEquals(claudeDeepseekUsage === sharedUsage, false);
  assert(claudeDeepseekUsage.includes("components.ProviderUsage"));
  assert(claudeDeepseekUsage.includes("components.DeepSeekDetails"));
  assert(claudeDeepseekUsage.includes("context.showDetails"));
  assert(claudeDeepseekUsage.includes('"provider.usage": ProviderUsageSlot'));
  assert(claudeDeepseekUsage.includes("components.ProviderSurface"));
  const details = Deno.readTextFileSync(
    new URL(
      "../../plugins/claude-deepseek/ui/DeepSeekDetails.tsx",
      import.meta.url,
    ),
  );
  assert(
    details.includes(
      "`/api/usage/${encodeURIComponent(usage.provider)}/activity?",
    ),
  );
  assert(details.includes("from \"../../../web/src/activityUsage\""));
  assert(details.includes("activityCacheStats"));
  assert(details.includes("usageActivityAgentLabel"));
  assert(details.includes("usageActivityModelLabel"));
  assert(details.includes("usageCacheMinHitLabel"));
  assert(details.includes("usageCacheIntervalLabel"));
  assert(details.includes("usageCacheOptionName"));
  assertEquals(details.includes("DEEPSEEK_CACHE_MIN_HIT_LABEL"), false);
  assertEquals(details.includes("DEEPSEEK_CACHE_BASE_INTERVAL_LABEL"), false);
  assertEquals(details.includes('agent === "claude"'), false);
  assertEquals(details.includes('agent === "codex"'), false);
  assertEquals(details.includes('family === "flash"'), false);
  assertEquals(details.includes('family === "pro"'), false);
  assertEquals(details.includes("DEEPSEEK_MODELS"), false);
  assertEquals(details.includes("/api/usage/deepseek/activity"), false);
});

for (const id of ["google", "apple", "cloudflare-email"]) {
  const oidc = await Deno.readTextFile(
    new URL(`../../examples/authentication/${id}/ui/index.js`, import.meta.url),
  );
  Deno.test(`${id} login module is a host-kit OIDC slot`, () => {
    assert(oidc.includes('context.kind !== "oidc"'));
    assert(oidc.includes('"login.method": OidcLogin') || oidc.includes("OidcLogin"));
    assert(oidc.includes("__COWBOY_PLUGIN_HOST"));
    assert(oidc.includes("host().auth?.startOidc"));
  });
}
