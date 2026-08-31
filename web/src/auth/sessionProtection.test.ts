import { assertEquals } from "jsr:@std/assert";
import type {
  ProductMe,
  ProductPasskeyServerPolicy,
  ProductSessionServerPolicy,
} from "./authApi.ts";
import {
  configuredSessionProtectionItems,
  currentSessionProtectionItems,
  sessionDeadlineLabel,
  sessionPolicyDuration,
} from "./sessionProtection.ts";

const passkeys: ProductPasskeyServerPolicy = {
  enabled: true,
  prompt_after_login: true,
  session_refresh_enabled: true,
};
const session: ProductSessionServerPolicy = {
  activity_sliding_enabled: true,
  idle_timeout_ms: 24 * 60 * 60 * 1_000,
  passkey_max_age_ms: 3 * 24 * 60 * 60 * 1_000,
  passkey_warning_ms: 30 * 60 * 1_000,
  primary_max_age_ms: 30 * 24 * 60 * 60 * 1_000,
  primary_warning_ms: 24 * 60 * 60 * 1_000,
};
const me: ProductMe = {
  account: "draven",
  role: "owner",
  passkey_count: 1,
  passkey_reauth_enabled: true,
  passkey_reauth_due_at_ms: 4 * 60 * 60 * 1_000,
  session_idle_due_at_ms: 60 * 60 * 1_000,
  primary_reauth_due_at_ms: 30 * 24 * 60 * 60 * 1_000,
  session_server_now_ms: 0,
};

Deno.test("session protection formats service policy without hiding configured fields", () => {
  assertEquals(sessionPolicyDuration(30 * 60 * 1_000), "30 minutes");
  assertEquals(sessionPolicyDuration(24 * 60 * 60 * 1_000), "1 day");
  assertEquals(configuredSessionProtectionItems(passkeys, session), [
    { label: "Activity extends idle timer", value: "On" },
    { label: "Idle timeout", value: "1 day" },
    { label: "Passkeys", value: "Enabled" },
    { label: "Prompt after sign-in", value: "On" },
    { label: "Passkey session extension", value: "Allowed" },
    { label: "Maximum Passkey interval", value: "3 days" },
    { label: "Passkey reminder", value: "30 minutes before" },
    { label: "Full sign-in limit", value: "30 days" },
    { label: "Full sign-in reminder", value: "1 day before" },
  ]);
});

Deno.test("current session protection distinguishes browser, Passkey, and full sign-in state", () => {
  assertEquals(currentSessionProtectionItems(me, passkeys, session, 0), [
    { label: "This browser", value: "Signed in as draven" },
    { label: "Idle sign-out", value: "Due in 1h" },
    { label: "Passkey check", value: "Due in 4h" },
    { label: "Full sign-in", value: "Due in 30d" },
  ]);
  assertEquals(
    currentSessionProtectionItems(
      { ...me, passkey_count: 0 },
      passkeys,
      session,
      0,
    )[2],
    { label: "Passkey check", value: "Set up a Passkey" },
  );
  assertEquals(
    currentSessionProtectionItems(
      { ...me, passkey_reauth_enabled: false },
      passkeys,
      session,
      0,
    )[2],
    { label: "Passkey check", value: "Off for this account" },
  );
  assertEquals(
    currentSessionProtectionItems(
      { ...me, passkey_reauth_due_at_ms: null },
      passkeys,
      session,
      0,
    )[2],
    { label: "Passkey check", value: "Verify this browser" },
  );
  assertEquals(
    sessionDeadlineLabel(23 * 60 * 60 * 1_000 + 59 * 60 * 1_000 + 1, 0),
    "Due in 1d",
  );
  assertEquals(sessionDeadlineLabel(-1, 0), "Required now");
});
