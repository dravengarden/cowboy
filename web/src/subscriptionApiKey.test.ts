import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { apiKeyErrorText, apiKeyStatus } from "./SubscriptionApiKey";
import { accountServes, providerDefault, validSubscriptionSnapshot } from "./subscriptionAccounts";

const credential = {
  account_id: "k", provider_id: "claude-deepseek", authentication_state: "ready",
  auth_generation: 2, distribution_state: "current", updated_at_ms: 1,
};

test("API key state is shown without the key", () => {
  assertEquals(apiKeyStatus(undefined).label, "No API key");
  assertEquals(apiKeyStatus(credential), { label: "API key saved", tone: "success", detail: "Delivered to Machines" });
  assertEquals(apiKeyStatus({ ...credential, distribution_state: "partial" }).detail, "Delivery to Machines pending");
  assertEquals(apiKeyStatus({ ...credential, authentication_state: "expired" }).tone, "warning");
  assertEquals(apiKeyErrorText("invalid_secret"), "Enter a single API key without line breaks.");
  assertEquals(apiKeyErrorText("sk-secret"), "The key was not saved. Try again.");
  assertEquals(apiKeyErrorText(undefined), "The key was not saved. Try again.");
});

test("portable scopes and defaults mirror the Service resolution", () => {
  const data = { registry: { defaults: { "deepseek-harness": "h", "claude-deepseek": "c" } as Record<string, string> },
    portable_scopes: [["claude-deepseek", "codex-deepseek", "deepseek-harness"]] };
  assertEquals(accountServes(data, "claude-deepseek", "codex-deepseek"), true);
  assertEquals(accountServes(data, "claude-deepseek", "codex"), false);
  assertEquals(accountServes(undefined, "codex", "codex"), true);
  assertEquals(providerDefault(data, "deepseek-harness"), "h");
  assertEquals(providerDefault(data, "codex-deepseek"), "c");
  assertEquals(providerDefault(data, "codex"), undefined);
  // An explicit Service-account choice is kept and is not overridden by a sibling.
  data.registry.defaults["codex-deepseek"] = "default";
  assertEquals(providerDefault(data, "codex-deepseek"), "default");
});

test("the account snapshot validates portable key state", () => {
  const snapshot = { dataset: { dataset_id: "d", user_id: "u" }, registry: { revision: 1, accounts: [], defaults: {} },
    session_admission: "enabled", portable_providers: ["claude-deepseek"], portable_scopes: [["claude-deepseek"]],
    portable_credentials: [credential] };
  assertEquals(validSubscriptionSnapshot(snapshot, "u"), true);
  assertEquals(validSubscriptionSnapshot({ ...snapshot, portable_scopes: ["claude-deepseek"] }, "u"), false);
  assertEquals(validSubscriptionSnapshot({ ...snapshot, portable_credentials: [{ account_id: "k" }] }, "u"), false);
});
