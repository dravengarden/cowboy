import { expect, test } from "bun:test";
import { accountIdentity, sessionAccountIdentity } from "./accountIdentity";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";

const inventory: SubscriptionSnapshot = {
  dataset: { dataset_id: "dataset", user_id: "owner" },
  registry: { revision: 1, accounts: [{ id: "otter", provider_id: "claude-code", label: "Claude · Otter", enabled: true }], defaults: { "claude-code": "otter" } },
  session_admission: "enabled",
  retired_default_providers: ["claude-code", "codex"],
};
test("session binding stays on the original account when creation default changes", () => {
  // A session of the retired legacy default card keeps a stable name.
  expect(sessionAccountIdentity(inventory, { provider: "claude-code", owner_user_id: "owner" }).short).toBe("Retired legacy account");
  expect(sessionAccountIdentity(inventory, { provider: "claude-code", provider_account_id: "otter", owner_user_id: "owner" }).short).toBe("Otter");
});
test("labels are owner and Provider scoped; missing named account never borrows default", () => {
  expect(sessionAccountIdentity(inventory, { provider: "claude-code", provider_account_id: "otter", owner_user_id: "other" }).label).toBe("Account not loaded");
  expect(accountIdentity(inventory, "codex", "otter").available).toBe(false);
  expect(accountIdentity(inventory, "claude-code", "deleted").label).toBe("Account unavailable");
  const disabled = structuredClone(inventory);
  disabled.registry.accounts[0]!.enabled = false;
  expect(accountIdentity(disabled, "claude-code", "otter")).toMatchObject({ label: "Claude · Otter", available: false });
});
test("unloaded inventory stays unknown without authorizing account controls", () => {
  expect(accountIdentity(undefined, "claude-code", "otter")).toMatchObject({
    id: "otter", label: "Account not loaded", short: "Account …", available: false, named: true,
  });
});
test("Calls use recorded execution binding; missing history is not today's default or parent account", () => {
  expect(accountIdentity(inventory, "codex", "default").short).toBe("Retired legacy account");
  expect(accountIdentity(inventory, "claude-code", undefined).label).toBe("Not recorded");
  expect(accountIdentity(inventory, "claude-code", null).label).toBe("Not recorded");
});
test("arbitrary custom account labels remain intact and shortening is display only", () => {
  const data = structuredClone(inventory);
  data.registry.accounts[0]!.label = "Research · weekend";
  expect(accountIdentity(data, "claude-code", "otter")).toMatchObject({ id: "otter", short: "weekend", label: "Research · weekend" });
  data.registry.accounts[0]!.label = "Research";
  expect(accountIdentity(data, "claude-code", "otter").short).toBe("Research");
});
test("only a card family's default is a retired legacy account", () => {
  expect(accountIdentity(inventory, "codex", "default")).toMatchObject({ label: "Retired legacy account", available: false });
  expect(accountIdentity(inventory, "future-provider", "default")).toMatchObject({ label: "Service account", available: true });
  // An older Controller names no retired Providers; its family table decides.
  const older = { ...inventory, retired_default_providers: undefined,
    browser_share_families: { version: 3, providers: { "claude-code": "anthropic", antigravity: "google" } } };
  expect(accountIdentity(older, "claude-code", "default").label).toBe("Retired legacy account");
  expect(accountIdentity(older, "antigravity", "default").label).toBe("Service account");
});
