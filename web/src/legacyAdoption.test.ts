import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { adoptionCard, adoptionErrorText, validAdoptionAttempt } from "./legacyAdoption";
import { type SubscriptionSnapshot, validSubscriptionSnapshot } from "./subscriptionAccounts";

const snapshot = (
  states: Record<string, "available" | "resumable" | "completed">,
): SubscriptionSnapshot => ({
  dataset: { dataset_id: "d", user_id: "u" },
  registry: { revision: 1, accounts: [], defaults: {} },
  session_admission: "enabled",
  portable_providers: ["claude-deepseek", "codex-deepseek", "deepseek-harness"],
  subscription_providers: ["claude-code", "codex", "grok"],
  legacy_adoption: Object.fromEntries(
    Object.entries(states).map(([card, state]) => [
      card,
      {
        provider_id: { openai: "codex", xai: "grok", deepseek: "claude-deepseek" }[card]!,
        state,
        account_id: `key-${card}`,
      },
    ]),
  ),
});

test("each legacy card upgrades from its primary Provider until it moved", () => {
  const data = snapshot({ openai: "available", xai: "resumable", deepseek: "completed" });
  assertEquals(adoptionCard(data, "codex"), "openai");
  assertEquals(adoptionCard(data, "grok"), "xai");
  // A completed move and scope siblings show no action; Anthropic has none.
  assertEquals(adoptionCard(data, "claude-deepseek"), undefined);
  assertEquals(adoptionCard(data, "codex-deepseek"), undefined);
  assertEquals(adoptionCard(data, "claude-code"), undefined);
  assertEquals(adoptionCard(undefined, "codex"), undefined);
  assertEquals(validSubscriptionSnapshot(data, "u"), true);
  assertEquals(
    validSubscriptionSnapshot(
      {
        ...data,
        legacy_adoption: { openai: { provider_id: "codex", state: "moved", account_id: "k" } },
      },
      "u",
    ),
    false,
  );
  assertEquals(validSubscriptionSnapshot({ ...data, subscription_providers: [1] }, "u"), false);
});

test("adoption requests carry only the operation", () => {
  assertEquals(validAdoptionAttempt({ operation: "0b1c-2d" }), true);
  for (const value of [
    null,
    [],
    { operation: "" },
    { operation: "a/b" },
    { operation: "op", machine: "ovh" },
  ]) {
    assertEquals(validAdoptionAttempt(value), false);
  }
  assertEquals(adoptionErrorText(undefined), "The upgrade did not finish. Try again.");
  assertEquals(
    adoptionErrorText("unknown_code"),
    "The upgrade did not finish (unknown_code). Try again.",
  );
});
