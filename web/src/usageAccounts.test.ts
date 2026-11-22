import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import type { SessionMeta } from "./protocol";
import { applyUsageHostPlugins, type UsageSnapshot, usageLimits } from "./usageLimits";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";
import { displayPlan, orderUsageAccounts, registeredUsageAccounts, sessionAccountQuota, usageCardManageUrl, usageResetTarget } from "./usageAccounts";
import { loadProviderCatalog, resetProviderCatalog } from "./providerCatalogRegistry";
import { providerUiManifestFixture } from "./providerUiContract.fixture";

const digest = `sha256:${"a".repeat(64)}`;
const registry: SubscriptionSnapshot = {
  dataset: { user_id: "owner", dataset_id: "dataset" },
  registry: { revision: 1, defaults: {}, accounts: [{ id: "otter", provider_id: "claude-code", label: "Claude · Otter", enabled: true }] },
  session_admission: "enabled",
};
const snapshot: UsageSnapshot = {
  refreshed_at_ms: 100, next_refresh_at_ms: 200, refresh_interval_ms: 100,
  providers: [{
    provider: "anthropic", source: "Anthropic", status: "available", observed_at_ms: 100,
    account: { plan: "Lynx" }, rate_limits: { rateLimits: { primary: { usedPercent: 38, windowDurationMins: 300 } } },
  }, { provider: "openai", source: "OpenAI", status: "available", observed_at_ms: 100 }],
};
const session: SessionMeta = {
  id: "s", cwd: "/workspace", title: "Session", status: "running",
  provider: "claude-code", provider_account_id: "otter", owner_user_id: "owner",
  provider_version: "1.0.0", provider_generation_digest: digest,
  usage: { used: 100, size: 1000, observed_at_ms: 200,
    raw: { _meta: { "future/quota": { utilization: 25, rateLimitType: "five_hour" } } } },
};

test("native observations remain account scoped, support refresh and preserve plan information", () => {
  setup();
  const observed = { account_id: "otter", provider_id: "claude-code", available: true, refreshing: false,
    usage: { provider: "anthropic", status: "available", source: "Anthropic", observed_at_ms: 300,
      account: { account: { planType: "max" } },
      rate_limits: { rateLimits: { primary: { usedPercent: 46, windowDurationMins: 300 } } } } };
  const cards = registeredUsageAccounts(snapshot, registry, [session], [observed]);
  assertEquals(usageLimits(cards[0].usage)[0].remaining, 54);
  assertEquals(cards[0].usage.account, observed.usage.account);
  assertEquals(cards[0].statusLabel, undefined);
  assertEquals(sessionAccountQuota(snapshot, registry, [session], session, [observed]), observed.usage);
  for (const wrong of [{ ...observed, account_id: "lynx" }, { ...observed, provider_id: "codex" }]) {
    assertEquals(registeredUsageAccounts(snapshot, registry, [], [wrong])[0].usage.rate_limits, undefined);
  }
  const disabled = structuredClone(registry);
  disabled.registry.accounts[0]!.enabled = false;
  assertEquals(registeredUsageAccounts(snapshot, disabled, [], [observed])[0].usage.rate_limits, undefined);
});
function setup() {
  applyUsageHostPlugins([
    { id: "claude-code", plugin_version: "1.0.0", artifact_digest: digest, usage: {
      account: "anthropic", product: "Anthropic", order: 0,
      limit_labels: [{ id: "five_hour", label: "5h", window_minutes: 300 }],
      session_rate_limits: { pointer: "/_meta/future~1quota", target: "rateLimits",
        required_number_fields: ["utilization"], required_string_fields: ["rateLimitType"] },
    } },
    { id: "codex", usage: { account: "openai", product: "OpenAI", order: 1 } },
  ]);
}
test("only registered accounts have usage cards; a Service observation is never one", () => {
  setup();
  // The legacy default cards are retired: the Service's own observations
  // (even a stale cached one) never become a card or a quota.
  const cards = registeredUsageAccounts(snapshot, registry, []);
  assertEquals(cards.map((c) => c.title), ["Claude · Otter"]);
  assertEquals(cards[0].usage.rate_limits, undefined);
  assertEquals(cards[0].usage.account, undefined);
  assertEquals(cards[0].usage.observed_at_ms, 0);
  assertEquals(cards[0].statusLabel, "NOT REPORTED");
});
test("session quota notices never borrow another account's exhausted window", async () => {
  const previousFetch = globalThis.fetch;
  const manifest = providerUiManifestFixture();
  manifest.id = session.provider;
  manifest.version = session.provider_version!;
  manifest.host.account_usage = { provider: "anthropic" };
  resetProviderCatalog();
  try {
    globalThis.fetch = async () => Response.json({
      providers: [{
        provider_id: manifest.id, provider_version: manifest.version,
        package_digest: digest, artifact_digest: digest, contract_fingerprint: digest,
        authentication_scope: "none-v1", release_state: "ready", publisher: manifest.publisher,
        supported_platforms: [{ os: "linux", architecture: "x86_64" }], manifest,
      }], authentications: [], authentication_executors: [], platform: { hosts: [] },
    });
    await loadProviderCatalog();
    setup();
    const exhausted = structuredClone(snapshot);
    exhausted.providers[0]!.rate_limits = { rateLimits: { primary: { usedPercent: 100, windowDurationMins: 300 } } };
    assertEquals(sessionAccountQuota(exhausted, registry, [], undefined), undefined);
    assertEquals(sessionAccountQuota(exhausted, registry, [], session)?.rate_limits, undefined);
    assertEquals(sessionAccountQuota(exhausted, registry, [session], { ...session, owner_user_id: "other" }), undefined);
    assertEquals(sessionAccountQuota(exhausted, registry, [session], { ...session, provider: "codex" }), undefined);
    const named = sessionAccountQuota(exhausted, registry, [session], session)!;
    assertEquals(usageLimits(named)[0].remaining, 75);
    // A retired legacy default session has no quota notice at all.
    const legacy = { ...session, provider_account_id: "default" };
    assertEquals(sessionAccountQuota(exhausted, registry, [session], legacy), undefined);
  } finally {
    globalThis.fetch = previousFetch;
    resetProviderCatalog();
  }
});
test("named quota requires exact account, owner, Provider and signed generation", () => {
  setup();
  const report = registeredUsageAccounts(snapshot, registry, [session])[0];
  assertEquals(report.usage.source, "Session report");
  assertEquals(report.usage.observed_at_ms, 200);
  assertEquals(usageLimits(report.usage)[0].remaining, 75);
  for (const changed of [
    { provider_account_id: "default" }, { provider_account_id: "other" }, { owner_user_id: "other" },
    { provider: "other" }, { provider_generation_digest: "other" }, { provider_version: undefined }, { closing: true },
    { usage: { ...session.usage!, raw: { used: 90, size: 100 } } },
    { usage: { ...session.usage!, raw: { _meta: { "future/quota": { utilization: "0.25", rateLimitType: "five_hour" } } } } },
  ]) {
    assertEquals(registeredUsageAccounts(snapshot, registry, [{ ...session, ...changed }])[0].usage.rate_limits, undefined);
  }
});
test("disabled and unsupported accounts remain visible without borrowed quota", () => {
  setup();
  for (const change of [{ enabled: false }, { provider_id: "unknown" }]) {
    const accounts = { ...registry, registry: { ...registry.registry, accounts: [{ ...registry.registry.accounts[0], ...change }] } };
    const card = registeredUsageAccounts(snapshot, accounts, [session]).find((c) => c.id === "otter")!;
    assertEquals(card.usage.rate_limits, undefined);
    assertEquals(card.title, "Claude · Otter");
  }
  const removed = { ...registry, registry: { ...registry.registry, accounts: [] } };
  assertEquals(registeredUsageAccounts(snapshot, removed, [session]).some((c) => c.id === "otter"), false);
});
test("revoked accounts leave the usage cards", () => {
  setup();
  const revoked = { ...registry, registry: { ...registry.registry,
    accounts: [{ ...registry.registry.accounts[0], enabled: false, revoking: true }] } };
  assertEquals(registeredUsageAccounts(snapshot, revoked, [session]).some((c) => c.id === "otter"), false);
});
test("newest valid quota wins; context-only reports do not erase or fabricate quota", () => {
  setup();
  const context = { ...session, usage: { ...session.usage!, observed_at_ms: 999, raw: { used: 900, size: 1000 } } };
  const card = registeredUsageAccounts(snapshot, registry, [context, session])[0];
  assertEquals(card.usage.observed_at_ms, 200);
  assertEquals(card.usage.activity, undefined);
  applyUsageHostPlugins([]);
  assertEquals(registeredUsageAccounts(snapshot, registry, [session]).find((c) => c.id === "otter")?.usage.rate_limits, undefined);
});
test("defaults label their card; legacy orders group accounts and explicit orders persist", () => {
  setup();
  const accounts = { ...registry, registry: { ...registry.registry, defaults: { "claude-code": "otter" },
    accounts: [...registry.registry.accounts, { id: "pika", provider_id: "codex", label: "OpenAI · Pika", enabled: true }] } };
  const cards = registeredUsageAccounts(snapshot, accounts, []);
  // Provider product order, the named accounts only.
  assertEquals(cards.map((c) => c.id), ["otter", "pika"]);
  assertEquals(cards[0].subtitle, "Anthropic · Default");
  assertEquals(cards[1].subtitle, "OpenAI");
  // An order saved while the legacy cards existed keeps grouping by Provider.
  assertEquals(orderUsageAccounts(cards, ["openai", "anthropic"]).map((c) => c.id), ["pika", "otter"]);
  assertEquals(orderUsageAccounts(cards, ["otter", "openai"]).map((c) => c.id), ["otter", "pika"]);
});
test("a login made before a Provider update asks to sign in again", () => {
  setup();
  const stale = { account_id: "otter", provider_id: "claude-code", refreshing: false, usage: null,
    reason: "native_login_outdated", message: "The Provider was updated after this account signed in." };
  const cards = registeredUsageAccounts(snapshot, registry, [], [stale]);
  assertEquals(cards[0].statusLabel, "SIGN IN AGAIN");
  assertEquals(cards[0].emptyMessage, stale.message);
});
test("sign-in states name the action instead of an unreported status", () => {
  setup();
  for (const [reason, label] of [["native_login_pending", "SIGN-IN PENDING"], ["native_login_required", "NOT SIGNED IN"], ["portable_credentials_required", "NOT SIGNED IN"]]) {
    const observation = { account_id: "otter", provider_id: "claude-code", refreshing: false, usage: null, reason, message: "m" };
    assertEquals(registeredUsageAccounts(snapshot, registry, [], [observation])[0].statusLabel, label);
  }
});
test("a Max report without a 5-hour window is shown as Ultra; nothing else is inferred", () => {
  const window = (minutes: number) => ({ primary: { usedPercent: 1, windowDurationMins: minutes } });
  const usage = (limits?: Record<string, unknown>) => ({ provider: "anthropic", status: "available", source: "", observed_at_ms: 1,
    ...(limits ? { rate_limits: { rateLimitsByLimitId: limits } } : {}) }) as never;
  assertEquals(displayPlan(usage({ "claude-seven_day": window(10080) }), "max"), "ultra");
  assertEquals(displayPlan(usage({ "claude-five_hour": window(300), "claude-seven_day": window(10080) }), "max"), "max");
  assertEquals(displayPlan(usage(), "max"), "max");
  assertEquals(displayPlan(usage({}), "max"), "max");
  assertEquals(displayPlan(usage({ "claude-seven_day": window(10080) }), "pro"), "pro");
  assertEquals(displayPlan(usage({ "claude-seven_day": window(10080) }), "ultra"), "ultra");
  assertEquals(displayPlan({ ...usage({ "x": window(10080) }), provider: "openai" } as never, "max"), "max");
});


test("a named-only Provider reports only its named accounts, never a Service card", () => {
  setup();
  const accounts: SubscriptionSnapshot = { ...registry, named_only_providers: ["claude-code"] };
  const cards = registeredUsageAccounts(snapshot, accounts, [session]);
  assertEquals(cards.map((card) => card.id), ["otter"]);
});

test("a named-only account's collected quota shows the plan its sessions reported", () => {
  setup();
  const control = {
    account_id: "otter", provider_id: "claude-code", admitted: true, state: "ok", revision: 1,
    fences: [], stop_confirmed: true, uncertain_effects: [], plan: { id: "g1-pro-tier", name: "Google AI Pro" },
  };
  const accounts = {
    ...registry, named_only_providers: ["claude-code"],
    account_controls: { loaded: true, accounts: [control] },
  } as SubscriptionSnapshot;
  const quota = {
    provider: "anthropic", status: "available", source: "Antigravity", observed_at_ms: 10,
    rate_limits: { rateLimitsByLimitId: { "0000-gemini-pro": { limitName: "Gemini Pro", primary: { usedPercent: 20, resetsAt: 2000, windowDurationMins: 300 } } } },
  };
  const observation = { account_id: "otter", provider_id: "claude-code", refreshing: false, usage: quota };
  const card = registeredUsageAccounts(snapshot, accounts, [], [observation]).find((item) => item.id === "otter");
  assertEquals(card?.statusLabel, "GOOGLE AI PRO");
  assertEquals(usageLimits(card?.usage).map((limit) => [limit.label, limit.remaining]), [["Gemini Pro · 5h", 80]]);
  // Without a collected quota the sign-in status still wins.
  const pending = registeredUsageAccounts(snapshot, accounts, [], [{ ...observation, usage: null, reason: "native_login_required" }]);
  assertEquals(pending.find((item) => item.id === "otter")?.statusLabel, "NOT SIGNED IN");
});

test("a named-only account is never asked to sign in again after a Provider update", () => {
  setup();
  const accounts: SubscriptionSnapshot = { ...registry, named_only_providers: ["claude-code"] };
  const outdated = { account_id: "otter", provider_id: "claude-code", refreshing: false, reason: "native_login_outdated",
    message: "The Provider was updated after this account signed in. Sign in again to use it and query its usage." };
  const card = registeredUsageAccounts(snapshot, accounts, [], [outdated]).find((item) => item.id === "otter");
  assertEquals(card?.statusLabel, "NOT REPORTED");
  assertEquals(card?.emptyMessage?.includes("Sign in again"), false);
  // A native Claude login keeps the explicit instruction.
  const claude = registeredUsageAccounts(snapshot, registry, [], [outdated]).find((item) => item.id === "otter");
  assertEquals(claude?.statusLabel, "SIGN IN AGAIN");
});

test("a named card owns its reset lane only when its own usage reports reset credits", () => {
  applyUsageHostPlugins([
    { id: "claude-code", usage: { account: "anthropic", product: "Anthropic", order: 0, reset: "anthropic" } },
  ]);
  const credits = { availableCount: 1, credits: [{ id: "grant:1", status: "available", title: "5h reset (1 left)" }] };
  const observed = { account_id: "otter", provider_id: "claude-code", available: true, refreshing: false,
    resets_supported: true,
    usage: { provider: "anthropic", status: "available", source: "Anthropic", observed_at_ms: 300,
      rate_limits: { rateLimitResetCredits: credits } } };
  const service = { ...snapshot, reset_schedules: { anthropic: { fire_at_ms: 500 } } };
  const [otter] = registeredUsageAccounts(service, registry, [], [observed]);
  // The named card spends only its own account's resets, never the Service's.
  assertEquals(usageResetTarget(otter!), { kind: "account", accountId: "otter" });
  assertEquals(usageCardManageUrl(otter!), "https://claude.ai/settings/usage");
  // Without its own credit report it neither borrows the Service lane nor links out.
  const quiet = { ...observed, usage: { ...observed.usage, rate_limits: {} } };
  const [plain] = registeredUsageAccounts(service, registry, [], [quiet]);
  assertEquals(usageResetTarget(plain!), undefined);
  // A named Claude card links to its plan page even without reset credits.
  assertEquals(usageCardManageUrl(plain!), "https://claude.ai/settings/usage");
  // And with no observation or Service snapshot at all.
  const bare = registeredUsageAccounts(null, registry, [], []).find((card) => card.id === "otter");
  assertEquals(usageCardManageUrl(bare!), "https://claude.ai/settings/usage");
  // A session-reported quota is never a reset lane.
  const [reported] = registeredUsageAccounts(service, registry, [session], []);
  assertEquals(usageResetTarget(reported!), undefined);
});
test("named resets appear only where the Service can spend them", () => {
  applyUsageHostPlugins([
    { id: "codex", usage: { account: "openai", product: "OpenAI", order: 1, reset: "codex" } },
    { id: "claude-deepseek", usage: { account: "deepseek", product: "DeepSeek", order: 3 } },
  ]);
  const accounts: SubscriptionSnapshot = { ...registry, registry: { revision: 1, defaults: {}, accounts: [
    { id: "pika", provider_id: "codex", label: "OpenAI · Pika", enabled: true },
    { id: "whale", provider_id: "codex-deepseek", label: "DeepSeek · Whale", enabled: true },
  ] } };
  const credits = { rateLimitResetCredits: { availableCount: 1, credits: [{ id: "credit-1", status: "available" }] } };
  const observation = (account_id: string, provider_id: string, provider: string, resets_supported: boolean) => ({
    account_id, provider_id, available: true, refreshing: false, resets_supported,
    usage: { provider, status: "available", source: provider, observed_at_ms: 300, rate_limits: credits },
  });
  const cards = registeredUsageAccounts(snapshot, accounts, [], [
    observation("pika", "codex", "openai", true),
    observation("whale", "codex-deepseek", "deepseek", false),
  ]);
  // A portable Codex account spends its own credits on its own lane.
  assertEquals(usageResetTarget(cards.find((card) => card.id === "pika")!), { kind: "account", accountId: "pika" });
  // DeepSeek declares no reset, whatever its report contains.
  assertEquals(usageResetTarget(cards.find((card) => card.id === "whale")!), undefined);
  // A Service that cannot spend the account's resets never shows the action.
  const older = registeredUsageAccounts(snapshot, accounts, [], [
    { ...observation("pika", "codex", "openai", true), resets_supported: undefined },
  ]);
  assertEquals(usageResetTarget(older.find((card) => card.id === "pika")!), undefined);
});
