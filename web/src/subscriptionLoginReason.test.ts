import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { subscriptionLoginReason } from "./SessionSubscription";

test("only definite sign-in states disable a subscription choice", () => {
  const observations = [
    {
      account_id: "mink",
      provider_id: "claude-code",
      reason: "native_login_outdated",
    },
    {
      account_id: "otter",
      provider_id: "claude-code",
      reason: "native_login_required",
    },
    {
      account_id: "pika",
      provider_id: "claude-code",
      reason: "native_login_pending",
    },
    {
      account_id: "fox",
      provider_id: "claude-code",
      reason: "machine_unavailable",
    },
    { account_id: "lynx", provider_id: "claude-code" },
  ];
  assertEquals(
    subscriptionLoginReason(observations, "claude-code", "mink"),
    "Sign in again: the Provider was updated",
  );
  assertEquals(
    subscriptionLoginReason(observations, "claude-code", "otter"),
    "Not signed in on a Machine",
  );
  assertEquals(
    subscriptionLoginReason(observations, "claude-code", "pika"),
    "Sign-in not finished: finish or cancel it",
  );
  // Transient Machine state and healthy logins stay selectable.
  assertEquals(
    subscriptionLoginReason(observations, "claude-code", "fox"),
    undefined,
  );
  assertEquals(
    subscriptionLoginReason(observations, "claude-code", "lynx"),
    undefined,
  );
  assertEquals(
    subscriptionLoginReason(observations, "claude-code", "default"),
    undefined,
  );
  // Another Provider's observation never applies.
  assertEquals(
    subscriptionLoginReason(observations, "codex", "mink"),
    undefined,
  );
});
