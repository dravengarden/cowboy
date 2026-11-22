import { test } from "bun:test";
import { assertEquals, assertThrows } from "../../tools/test-assert.ts";
import { subscriptionLoginsPage } from "./SubscriptionLogins";
import { obsoletePendingSubscription, subscriptionWriteRejected, validPendingSubscription, validSubscriptionLabel, validSubscriptionSnapshot } from "./SubscriptionManagement";
import { accountDefaults, withoutLegacyShareMembers } from "./subscriptionAccounts";

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
  // A definitive refusal of a retired default ends the retry.
  assertEquals(subscriptionWriteRejected(422, "named_account_required"), true);
});

test("a pending change saved for a retired legacy card is discarded, never retried", () => {
  const input = { operation: "op", dataset: "dataset", expected_revision: 3 };
  for (const change of [{ type: "set_legacy_labels", labels: { "claude-code": "Claude · Lynx" } },
    { type: "set_legacy_browser_enabled", card: "anthropic", enabled: true },
    { type: "set_browser_share", member: { card: "anthropic" }, share_id: "share" }]) {
    assertEquals(obsoletePendingSubscription({ ...input, change }), true);
  }
  for (const change of [{ type: "set_enabled", id: "account", enabled: false },
    { type: "set_browser_share", member: { account_id: "account" }, share_id: null }]) {
    assertEquals(obsoletePendingSubscription({ ...input, change }), false);
  }
  assertEquals(obsoletePendingSubscription(null), false);
});

test("an older Controller's legacy card share member is read as a bound family", () => {
  const share = { share_id: "share", label: "Google", created_ms: 1, revision: 2, deleting: false,
    members: [{ member: { account_id: "otter" }, family: "anthropic", account_id: "otter" },
      { member: { card: "openai" }, family: "openai", account_id: "pika-key" }] };
  const input = { dataset: { dataset_id: "dataset", user_id: "owner" }, session_admission: "enabled", browser_shares: [share],
    registry: { revision: 0, defaults: {}, accounts: [{ id: "otter", provider_id: "claude-code", label: "Otter", enabled: true }] } };
  assertEquals(validSubscriptionSnapshot(input, "owner"), true);
  if (!validSubscriptionSnapshot(input, "owner")) return;
  const read = withoutLegacyShareMembers(input).browser_shares![0]!;
  assertEquals(read.members.map((row) => row.account_id), ["otter"]);
  assertEquals(read.bound, [{ family: "openai", account_id: "pika-key" }]);
  // A malformed member still rejects the snapshot.
  const garbled = { ...input, browser_shares: [{ ...share, members: [{ member: { card: "" }, family: "openai", account_id: "x" }] }] };
  assertEquals(validSubscriptionSnapshot(garbled, "owner"), false);
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


test("an adopted shared-scope account lists every default it must clear, its own first", () => {
  const snapshot = { registry: { defaults: { "codex-deepseek": "mole", "claude-deepseek": "mole", "deepseek-harness": "mole", codex: "pika" } } };
  assertEquals(accountDefaults(snapshot, { id: "mole", provider_id: "claude-deepseek" }), ["claude-deepseek", "codex-deepseek", "deepseek-harness"]);
  assertEquals(accountDefaults(snapshot, { id: "otter", provider_id: "claude-code" }), []);
});
