import { test } from "bun:test";
import { assertEquals, assertThrows } from "@std/assert";
import { subscriptionLoginsPage } from "./SubscriptionLogins";
import { subscriptionWriteRejected, validPendingSubscription, validSubscriptionLabel, validSubscriptionSnapshot } from "./SubscriptionManagement";
import { legacySubscriptionLabel, validLegacyLabels } from "./subscriptionDisplay";

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
  const legacy = { ...input, change: { type: "set_legacy_labels", labels: { "claude-code": "Claude · Lynx" } } };
  assertEquals(validPendingSubscription(legacy), true);
  assertEquals(validPendingSubscription({ ...legacy, change: { ...legacy.change, owner: "other" } }), false);
  for (const value of [null, { ...input, expected_revision: -1 }, { ...input, token: "secret" },
    { ...input, change: { ...input.change, enabled: "false" } }, { ...input, change: { type: "login" } }]) {
    assertEquals(validPendingSubscription(value), false);
  }
});

test("subscription responses must belong to the owner and contain valid account records", () => {
  const input = { dataset: { dataset_id: "dataset", user_id: "owner" }, session_admission: "not_enabled",
    registry: { revision: 0, defaults: {}, accounts: [{ id: "account", provider_id: "codex", label: "Personal", enabled: true }] } };
  assertEquals(validSubscriptionSnapshot(input, "owner"), true);
  const display = { scope: "subscriptions" as const, revision: 2, values: { legacy_labels: { "claude-code": "Claude · Lynx" } } };
  assertEquals(validSubscriptionSnapshot({ ...input, display_preferences: display }, "owner"), true);
  assertEquals(validSubscriptionSnapshot({ ...input, display_preferences: { ...display, revision: -1 } }, "owner"), false);
  assertEquals(legacySubscriptionLabel(display, "claude-code"), "Claude · Lynx");
  assertEquals(legacySubscriptionLabel(display, "codex"), "Existing account");
  assertEquals(legacySubscriptionLabel(undefined, "claude-code"), "Existing account");
  assertEquals(legacySubscriptionLabel(display, "constructor"), "Existing account");
  assertEquals(validSubscriptionSnapshot(input, "other"), false);
  assertEquals(validSubscriptionSnapshot({ ...input, registry: { ...input.registry, accounts: [null] } }, "owner"), false);
  assertEquals(validSubscriptionSnapshot({ ...input, registry: { ...input.registry, defaults: [] } }, "owner"), false);
});

test("legacy labels reject malformed provider maps and authority fields", () => {
  for (const value of [null, [], { "claude-code": null }, { "claude-code": " padded " }, { "bad/provider": "Name" },
    { "claude-code": "界".repeat(43) }, { "claude-code": "a\nb" }, { "claude-code": { token: "secret" } },
    Object.fromEntries(Array.from({ length: 129 }, (_, index) => [`provider-${index}`, "Name"]))]) {
    assertEquals(validLegacyLabels(value), false);
  }
});
