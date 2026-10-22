import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { SessionMeta } from "./protocol";
import { applyUsageHostPlugins, type UsageSnapshot, usageLimits } from "./usageLimits";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";
import { orderUsageAccounts, registeredUsageAccounts, sessionAccountQuota } from "./usageAccounts";
import { loadProviderCatalog, resetProviderCatalog } from "./providerCatalogRegistry";
import { providerUiManifestFixture } from "./providerUiContract.fixture";

const digest = `sha256:${"a".repeat(64)}`;
const registry: SubscriptionSnapshot = {
  dataset: { user_id: "owner", dataset_id: "dataset" },
  registry: { revision: 1, defaults: {}, accounts: [{ id: "otter", provider_id: "claude-code", label: "Claude · Otter", enabled: true }] },
  display_preferences: { scope: "subscriptions", revision: 1, values: { legacy_labels: { "claude-code": "Claude · Lynx" } } },
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
  assertEquals(usageLimits(cards[1].usage)[0].remaining, 54);
  assertEquals(cards[1].usage.account, observed.usage.account);
  assertEquals(cards[1].statusLabel, undefined);
  assertEquals(cards[1].collector, undefined); // Native quota never exposes Service reset/placement.
  assertEquals(cards[0].usage, snapshot.providers[0]);
  assertEquals(sessionAccountQuota(snapshot, registry, [session], session, [observed]), observed.usage);
  for (const wrong of [{ ...observed, account_id: "lynx" }, { ...observed, provider_id: "codex" }]) {
    assertEquals(registeredUsageAccounts(snapshot, registry, [], [wrong])[1].usage.rate_limits, undefined);
  }
  const disabled = structuredClone(registry);
  disabled.registry.accounts[0]!.enabled = false;
  assertEquals(registeredUsageAccounts(snapshot, disabled, [], [observed])[1].usage.rate_limits, undefined);
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
test("registered accounts are distinct and never inherit Service usage or controls", () => {
  setup();
  const cards = registeredUsageAccounts(snapshot, registry, []);
  assertEquals(cards.map((c) => c.title), ["Claude · Lynx", "Claude · Otter", "OpenAI"]);
  assertEquals(cards[0].subtitle, "Anthropic · Default");
  assertEquals(cards[0].collector, "anthropic");
  assertEquals(cards[1].collector, undefined);
  assertEquals(cards[1].usage.rate_limits, undefined);
  assertEquals(cards[1].usage.account, undefined);
  assertEquals(cards[1].usage.observed_at_ms, 0);
  assertEquals(cards[1].statusLabel, "NOT REPORTED");
  assertEquals(usageLimits(cards[0].usage)[0].remaining, 62);
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
    const legacy = { ...session, provider_account_id: "default" };
    assertEquals(usageLimits(sessionAccountQuota(exhausted, registry, [session], legacy)!)[0].remaining, 0);
  } finally {
    globalThis.fetch = previousFetch;
    resetProviderCatalog();
  }
});
test("named quota requires exact account, owner, Provider and signed generation", () => {
  setup();
  const report = registeredUsageAccounts(snapshot, registry, [session])[1];
  assertEquals(report.usage.source, "Session report");
  assertEquals(report.usage.observed_at_ms, 200);
  assertEquals(usageLimits(report.usage)[0].remaining, 75);
  for (const changed of [
    { provider_account_id: "default" }, { provider_account_id: "other" }, { owner_user_id: "other" },
    { provider: "other" }, { provider_generation_digest: "other" }, { provider_version: undefined }, { closing: true },
    { usage: { ...session.usage!, raw: { used: 90, size: 100 } } },
    { usage: { ...session.usage!, raw: { _meta: { "future/quota": { utilization: "0.25", rateLimitType: "five_hour" } } } } },
  ]) {
    assertEquals(registeredUsageAccounts(snapshot, registry, [{ ...session, ...changed }])[1].usage.rate_limits, undefined);
  }
  assertEquals(registeredUsageAccounts(snapshot, registry, [session])[0].usage, snapshot.providers[0]);
});
test("disabled, revoked and unsupported accounts remain visible without borrowed quota", () => {
  setup();
  for (const change of [{ enabled: false }, { revoking: true }, { provider_id: "unknown" }]) {
    const accounts = { ...registry, registry: { ...registry.registry, accounts: [{ ...registry.registry.accounts[0], ...change }] } };
    const card = registeredUsageAccounts(snapshot, accounts, [session]).find((c) => c.id === "otter")!;
    assertEquals(card.usage.rate_limits, undefined);
    assertEquals(card.collector, undefined);
    assertEquals(card.title, "Claude · Otter");
  }
  const removed = { ...registry, registry: { ...registry.registry, accounts: [] } };
  assertEquals(registeredUsageAccounts(snapshot, removed, [session]).some((c) => c.id === "otter"), false);
});
test("newest valid quota wins; context-only reports do not erase or fabricate quota", () => {
  setup();
  const context = { ...session, usage: { ...session.usage!, observed_at_ms: 999, raw: { used: 900, size: 1000 } } };
  const card = registeredUsageAccounts(snapshot, registry, [context, session])[1];
  assertEquals(card.usage.observed_at_ms, 200);
  assertEquals(card.usage.activity, undefined);
  applyUsageHostPlugins([]);
  assertEquals(registeredUsageAccounts(snapshot, registry, [session]).find((c) => c.id === "otter")?.usage.rate_limits, undefined);
});
test("default changes never rebind Service data; legacy orders group accounts and explicit orders persist", () => {
  setup();
  const accounts = { ...registry, registry: { ...registry.registry, defaults: { "claude-code": "otter" } } };
  const cards = registeredUsageAccounts(snapshot, accounts, []);
  assertEquals(cards[0].usage, snapshot.providers[0]);
  assertEquals(cards[0].subtitle, "Anthropic");
  assertEquals(cards[1].subtitle, "Anthropic · Default");
  assertEquals(orderUsageAccounts(cards, ["anthropic", "openai"]).map((c) => c.id), ["anthropic", "otter", "openai"]);
  assertEquals(orderUsageAccounts(cards, ["openai", "otter", "anthropic"]).map((c) => c.id), ["openai", "otter", "anthropic"]);
});
test("a login made before a Provider update asks to sign in again", () => {
  setup();
  const stale = { account_id: "otter", provider_id: "claude-code", refreshing: false, usage: null,
    reason: "native_login_outdated", message: "The Provider was updated after this account signed in." };
  const cards = registeredUsageAccounts(snapshot, registry, [], [stale]);
  assertEquals(cards[1].statusLabel, "SIGN IN AGAIN");
  assertEquals(cards[1].emptyMessage, stale.message);
});
