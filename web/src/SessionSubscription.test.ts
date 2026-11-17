import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { subscriptionAllowed } from "./SessionSubscription";

test("subscription selection rejects unavailable defaults and never falls back across providers", () => {
  const data = { registry: {
    accounts: [{ id: "a", provider_id: "claude-code", label: "Personal", enabled: true }],
    defaults: { "claude-code": "a" },
  }, session_admission: "not_enabled", session_providers: [] as string[] };
  assertEquals(subscriptionAllowed(undefined, "claude-code", ""), false);
  assertEquals(subscriptionAllowed(data, "claude-code", ""), false);
  assertEquals(subscriptionAllowed(data, "claude-code", "default"), true);
  assertEquals(subscriptionAllowed(data, "claude-code", "a"), false);
  data.session_admission = "enabled";
  assertEquals(subscriptionAllowed(data, "claude-code", ""), false);
  data.session_providers = ["claude-code"];
  assertEquals(subscriptionAllowed(data, "claude-code", ""), true);
  assertEquals(subscriptionAllowed(data, "codex", "a"), false);
  data.registry.accounts[0]!.enabled = false;
  assertEquals(subscriptionAllowed(data, "claude-code", "a"), false);
  assertEquals(subscriptionAllowed(data, "claude-code", "missing"), false);
});

test("a global enabled flag cannot enable an unsupported provider", () => {
  const data = { registry: { accounts: [{ id: "c", provider_id: "codex", label: "Codex", enabled: true }],
    defaults: { codex: "c" } }, session_admission: "enabled", session_providers: ["claude-code"] };
  assertEquals(subscriptionAllowed(data, "codex", ""), false);
  assertEquals(subscriptionAllowed(data, "codex", "c"), false);
  assertEquals(subscriptionAllowed(data, "codex", "default"), true);
});

test("one portable account serves its whole credential scope, never other Providers", () => {
  const scope = ["claude-deepseek", "codex-deepseek", "deepseek-harness"];
  const data = { registry: {
    accounts: [{ id: "k", provider_id: "claude-deepseek", label: "DeepSeek", enabled: true }],
    defaults: { "claude-deepseek": "k" } as Record<string, string>,
  }, session_admission: "enabled", session_providers: [...scope, "codex"], portable_scopes: [scope] };
  for (const provider of scope) {
    assertEquals(subscriptionAllowed(data, provider, "k"), true);
    // The scope member default is inherited, as the Service resolves it.
    assertEquals(subscriptionAllowed(data, provider, ""), true);
  }
  assertEquals(subscriptionAllowed(data, "codex", "k"), false);
  assertEquals(subscriptionAllowed({ ...data, portable_scopes: [] }, "codex-deepseek", "k"), false);
  data.registry.accounts[0]!.enabled = false;
  // An unavailable inherited default refuses instead of using the Service account.
  assertEquals(subscriptionAllowed(data, "deepseek-harness", ""), false);
});
