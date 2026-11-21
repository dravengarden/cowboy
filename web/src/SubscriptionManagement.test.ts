import { test } from "bun:test";
import { assertEquals, assertThrows } from "../../tools/test-assert.ts";
import { subscriptionLoginsPage } from "./SubscriptionLogins";
import { subscriptionWriteRejected, validPendingSubscription, validSubscriptionLabel, validSubscriptionSnapshot } from "./SubscriptionManagement";

test("login discovery rejects mixed accounts, duplicate Machines and nonadvancing cursors", () => {
  const scope = { owner: "owner", dataset: "dataset", account: "account", provider: "claude-code" };
  const login = { account_id: "account", provider_id: "claude-code", machine_id: "b", request_id: "request", status: "active", cancel_requested: false };
  const page = { account_id: "account", provider_id: "claude-code", logins: [login], next_after_machine: "b" };
  assertEquals(subscriptionLoginsPage(page, scope, "a").logins[0], login);
  for (const invalid of [{ ...page, account_id: "other" }, { ...page, logins: [{ ...login, account_id: "other" }] },
    { ...page, logins: [login, login] }, { ...page, next_after_machine: "a" }, { ...page, logins: [] }]) {
    assertThrows(() => subscriptionLoginsPage(invalid, scope, "a"));
  }
  assertThrows(() => subscriptionLoginsPage(page, scope, "b"));
});

test("subscription labels obey UTF-8 byte limits and reject control characters", () => {
  for (const label of ["Personal Max", " 个 人 ", "界".repeat(42)]) assertEquals(validSubscriptionLabel(label), true);
  for (const label of [" ", "界".repeat(43), "a\nb", "a\u0085b", "a\0b"]) assertEquals(validSubscriptionLabel(label), false);
});

test("uncertain subscription errors never discard the original operation", () => {
  assertEquals(subscriptionWriteRejected(422, "account_is_default"), true);
  for (const code of [undefined, "outcome_unknown", "operation_reused", "new_server_error"]) {
    assertEquals(subscriptionWriteRejected(422, code), false);
  }
  assertEquals(subscriptionWriteRejected(503, "account_is_default"), false);
});

test("persisted subscription retries reject malformed or expanded actions", () => {
  const input = { operation: "same-operation", dataset: "dataset", expected_revision: 0,
    change: { type: "set_enabled", id: "account", enabled: false } };
  assertEquals(validPendingSubscription(input), true);
  assertEquals(validPendingSubscription({ ...input, change: { ...input.change, owner: "other" } }), false);
  // The retired legacy card writes are no pending change any more.
  for (const change of [{ type: "set_legacy_labels", labels: { "claude-code": "Claude · Lynx" } },
    { type: "set_legacy_browser_enabled", card: "anthropic", enabled: true }]) {
    assertEquals(validPendingSubscription({ ...input, change }), false);
  }
  for (const value of [null, { ...input, expected_revision: -1 }, { ...input, token: "secret" },
    { ...input, change: { ...input.change, enabled: "false" } }, { ...input, change: { type: "login" } }]) {
    assertEquals(validPendingSubscription(value), false);
  }
});

test("subscription responses must belong to the owner and contain valid account records", () => {
  const input = { dataset: { dataset_id: "dataset", user_id: "owner" }, session_admission: "not_enabled",
    registry: { revision: 0, defaults: {}, accounts: [{ id: "account", provider_id: "codex", label: "Personal", enabled: true }] } };
  assertEquals(validSubscriptionSnapshot(input, "owner"), true);
  // A Controller from before the retirement still sends its legacy fields.
  assertEquals(validSubscriptionSnapshot({ ...input, display_preferences: { scope: "subscriptions" }, legacy_browsers: [{}] }, "owner"), true);
  assertEquals(validSubscriptionSnapshot({ ...input, retired_default_providers: ["codex"] }, "owner"), true);
  assertEquals(validSubscriptionSnapshot({ ...input, retired_default_providers: [1] }, "owner"), false);
  assertEquals(validSubscriptionSnapshot(input, "other"), false);
  assertEquals(validSubscriptionSnapshot({ ...input, registry: { ...input.registry, accounts: [null] } }, "owner"), false);
  assertEquals(validSubscriptionSnapshot({ ...input, registry: { ...input.registry, defaults: [] } }, "owner"), false);
});

