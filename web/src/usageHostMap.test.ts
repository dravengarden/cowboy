import { assert, assertEquals } from "jsr:@std/assert";
import { bundledHostPlugins } from "./bundledHostPlugins.ts";
import {
  applyUsageHostPlugins,
  usageAvailableStatus,
  usageCardOrder,
  usageEmptyMessage,
  usageErrorAuth,
  usageErrorConfig,
  usageErrorFetch,
  usageErrorKind,
  usageLimitLabel,
  usageLimitParser,
  usageLimitRowId,
  usageOmitEmptyLimits,
  usagePluginId,
  usageProductLabel,
  usageResetId,
  usageTopBarWindowMinutes,
  usageActivityAgentIds,
  usageActivityAgentLabel,
  usageActivityAgents,
  usageActivityModelIds,
  usageActivityModelLabel,
  usageActivityModels,
  usageCacheIntervalLabel,
  usageCacheIntervalMs,
  usageCacheMinHitLabel,
  usageCacheMinHitTokens,
  usageCacheOptionName,
  usageWidgetBalanceLabel,
  usageWidgetKind,
  usageWidgetShape,
  usageWidgetSpendLabel,
  usageWidgetWindow,
} from "./usageHostMap.ts";

Deno.test("first-party usage maps are generated from bundled host.json", () => {
  const source = Deno.readTextFileSync(new URL("./usageHostMap.ts", import.meta.url));
  assert(source.includes("bundledHostPlugins"));
  assertEquals(source.includes("FALLBACK_USAGE_PLUGIN_IDS"), false);
  assertEquals(source.includes('provider = "deepseek"'), false);
  assertEquals(
    bundledHostPlugins.map((host) => host.id),
    [
      "codex",
      "codex-deepseek",
      "grok",
      "gemini",
      "claude-code",
      "claude-deepseek",
    ],
  );
});

Deno.test("usage plugin ids map account providers onto agent plugins", () => {
  assertEquals(usagePluginId("openai"), "codex");
  assertEquals(usagePluginId("xai"), "grok");
  assertEquals(usagePluginId("anthropic"), "claude-code");
  assertEquals(usagePluginId("deepseek"), "claude-deepseek");
  assertEquals(usagePluginId("gemini"), "gemini");
  assertEquals(usagePluginId("future-b"), "future-b");
});

Deno.test("activated host plugins overlay usage account mapping", () => {
  try {
    applyUsageHostPlugins([
      { id: "custom-grok", usage_account: "xai" },
      { id: "password", slots: ["login.method"] },
    ]);
    assertEquals(usagePluginId("xai"), "custom-grok");
    assertEquals(usagePluginId("openai"), "codex");
  } finally {
    applyUsageHostPlugins([]);
  }
  assertEquals(usagePluginId("xai"), "grok");
});

Deno.test("host usage specs overlay reset ids and product labels", () => {
  try {
    applyUsageHostPlugins([{
      id: "grok",
      usage: {
        account: "xai",
        collector: "xai-billing",
        reset: "xai",
        product: "Grok Build",
      },
    }]);
    assertEquals(usagePluginId("xai"), "grok");
    assertEquals(usageResetId("xai"), "xai");
    assertEquals(usageResetId("openai"), "codex");
    assertEquals(usageProductLabel("xai"), "Grok Build");
    assertEquals(usageProductLabel("openai"), "OpenAI");
  } finally {
    applyUsageHostPlugins([]);
  }
  assertEquals(usageProductLabel("xai"), "xAI");
});

Deno.test("host usage specs overlay parser, order, errors, and top-bar windows", () => {
  try {
    applyUsageHostPlugins([{
      id: "custom-codex",
      usage: {
        account: "openai",
        parser: "generic-buckets",
        error: "openai-auth",
        order: 9,
        top_bar_windows: [60],
        widget: "openai-weekly",
        empty: "Custom empty",
        product: "Custom OpenAI",
        available_status: "PLAN",
        omit_empty_limits: true,
        widget_shape: "percent",
        widget_window: 1440,
        error_auth: "Custom OpenAI auth",
        error_config: "Custom OpenAI config",
        error_fetch: "Custom OpenAI fetch",
      },
    }]);
    assertEquals(usageLimitParser("openai"), "generic-buckets");
    assertEquals(usageErrorKind("openai"), "openai-auth");
    assertEquals(usageCardOrder("openai"), 9);
    assertEquals(usageTopBarWindowMinutes("openai"), [60]);
    assertEquals(usageWidgetKind("openai"), "openai-weekly");
    assertEquals(usageEmptyMessage("openai"), "Custom empty");
    assertEquals(usageAvailableStatus("openai"), "PLAN");
    assertEquals(usageOmitEmptyLimits("openai"), true);
    assertEquals(usageWidgetShape("openai"), "percent");
    assertEquals(usageWidgetWindow("openai"), 1440);
    assertEquals(usageErrorAuth("openai"), "Custom OpenAI auth");
    assertEquals(usageErrorConfig("openai"), "Custom OpenAI config");
    assertEquals(usageErrorFetch("openai"), "Custom OpenAI fetch");
    assertEquals(usageLimitParser("xai"), "xai-credits");
    assertEquals(usageCardOrder("xai"), 1);
    assertEquals(usageTopBarWindowMinutes("xai"), undefined);
  } finally {
    applyUsageHostPlugins([]);
  }
  assertEquals(usageLimitParser("xai"), "xai-credits");
  assertEquals(usageLimitParser("anthropic"), "anthropic-utilization");
  assertEquals(usageLimitParser("gemini"), "generic-buckets");
  assertEquals(usageErrorKind("openai"), "openai-auth");
  assertEquals(usageErrorKind("xai"), "xai-billing");
  assertEquals(usageErrorKind("gemini"), "raw");
  assertEquals(usageCardOrder("openai"), 0);
  assertEquals(usageTopBarWindowMinutes("openai"), [300, 10080]);
  assertEquals(usageWidgetKind("openai"), "openai-weekly");
  assertEquals(usageWidgetKind("xai"), "xai-included");
  assertEquals(usageWidgetKind("deepseek"), "deepseek-balance");
  assertEquals(usageWidgetKind("gemini"), "none");
  assertEquals(
    usageEmptyMessage("anthropic"),
    "Waiting for session activity. Plan limits appear after the Provider reports them.",
  );
  assertEquals(
    usageEmptyMessage("gemini"),
    "Account quota is not exposed by this Provider.",
  );
  assertEquals(usageEmptyMessage("openai"), undefined);
  assertEquals(usageAvailableStatus("deepseek"), "API");
  assertEquals(usageAvailableStatus("openai"), undefined);
  assertEquals(usageOmitEmptyLimits("deepseek"), true);
  assertEquals(usageOmitEmptyLimits("openai"), false);
  assertEquals(usageWidgetShape("openai"), "percent");
  assertEquals(usageWidgetShape("xai"), "percent");
  assertEquals(usageWidgetShape("deepseek"), "balance");
  assertEquals(usageWidgetShape("gemini"), "none");
  assertEquals(usageWidgetWindow("openai"), 10080);
  assertEquals(usageWidgetWindow("xai"), undefined);
  assertEquals(
    usageErrorAuth("openai"),
    "OpenAI usage authorization expired. Sign in to Codex again.",
  );
  assertEquals(
    usageErrorAuth("xai"),
    "Sign in to Grok Build in Machines, then refresh xAI usage.",
  );
  assertEquals(usageErrorAuth("gemini"), undefined);
  assertEquals(
    usageErrorConfig("xai"),
    "Grok Build usage is not configured on this Machine.",
  );
  assertEquals(
    usageErrorFetch("xai"),
    "Grok Build could not fetch xAI usage.",
  );
  assertEquals(usageLimitRowId("anthropic", "five_hour"), "claude-five_hour");
  assertEquals(usageLimitLabel("anthropic", "five_hour"), {
    label: "5h",
    windowMinutes: 300,
  });
  assertEquals(usageLimitLabel("anthropic", "unknown"), { label: "Plan usage" });
  assertEquals(usageWidgetBalanceLabel("deepseek"), "Balance");
  assertEquals(usageWidgetSpendLabel("deepseek"), "24h spend");
  assertEquals(usageWidgetBalanceLabel("openai"), "Balance");
  assertEquals(usageActivityAgentIds(), ["codex", "claude"]);
  assertEquals(usageActivityAgentLabel("claude"), "Claude Code");
  assertEquals(usageActivityAgents("gemini"), []);
  assertEquals(usageActivityModelIds(), ["flash", "pro"]);
  assertEquals(usageActivityModelLabel("flash"), "Flash");
  assertEquals(usageActivityModels("gemini"), []);
  assertEquals(usageCacheMinHitTokens(), 64_000);
  assertEquals(usageCacheMinHitLabel(), "64K");
  assertEquals(usageCacheIntervalMs(), 28_800_000);
  assertEquals(usageCacheIntervalLabel(), "8h");
  assertEquals(usageCacheOptionName(), "Cache protection");
  assertEquals(usageCacheMinHitTokens("gemini"), 0);
  assertEquals(usageCacheOptionName("gemini"), "");
});

Deno.test("host usage specs overlay cache-protection thresholds", () => {
  try {
    applyUsageHostPlugins([{
      id: "claude-deepseek",
      usage: {
        account: "deepseek",
        cache_protection: {
          min_hit_tokens: 32_000,
          min_hit_label: "32K",
          interval_ms: 3_600_000,
          interval_label: "1h",
          option_name: "Prompt cache",
          option_description: "Custom cache protection copy.",
          option_on: "On",
          option_off: "Off",
        },
      },
    }]);
    assertEquals(usageCacheMinHitTokens(), 32_000);
    assertEquals(usageCacheMinHitLabel(), "32K");
    assertEquals(usageCacheIntervalMs(), 3_600_000);
    assertEquals(usageCacheIntervalLabel(), "1h");
    assertEquals(usageCacheOptionName(), "Prompt cache");
  } finally {
    applyUsageHostPlugins([]);
  }
  assertEquals(usageCacheMinHitTokens(), 64_000);
  assertEquals(usageCacheIntervalLabel(), "8h");
});

Deno.test("host usage specs overlay activity model families", () => {
  try {
    applyUsageHostPlugins([{
      id: "claude-deepseek",
      usage: {
        account: "deepseek",
        activity_models: [{ id: "flash", label: "Lite" }],
      },
    }]);
    assertEquals(usageActivityModelIds(), ["flash"]);
    assertEquals(usageActivityModelLabel("flash"), "Lite");
    assertEquals(usageActivityModelLabel("pro"), "pro");
  } finally {
    applyUsageHostPlugins([]);
  }
  assertEquals(usageActivityModelIds(), ["flash", "pro"]);
  assertEquals(usageActivityModelLabel("pro"), "Pro");
});

Deno.test("reset ids index the plugin-keyed schedule map", () => {
  const schedules: Record<string, { fire_at_ms: number }> = {
    codex: { fire_at_ms: 100 },
    xai: { fire_at_ms: 200 },
  };
  assertEquals(schedules[usageResetId("openai") ?? ""]?.fire_at_ms, 100);
  assertEquals(schedules[usageResetId("xai") ?? ""]?.fire_at_ms, 200);
  assertEquals(schedules[usageResetId("gemini") ?? ""], undefined);
});
