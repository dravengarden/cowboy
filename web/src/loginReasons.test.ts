import { test } from "bun:test";
import { assertEquals, assertNotEquals } from "@std/assert";
import { loginReasonText } from "./loginReasons";

test("login readiness codes never reach people raw", () => {
  for (
    const code of [
      "account_disabled",
      "machine_not_permitted",
      "machine_unavailable",
      "machine_account_protocol_required",
      "machine_recipient_unavailable",
      "provider_not_installed",
      "release_unavailable",
      "native_identity_verification_unavailable",
    ]
  ) {
    const text = loginReasonText(code);
    assertEquals(text.includes("_"), false, code);
    assertNotEquals(text, loginReasonText("future_code"), code);
  }
  assertEquals(
    loginReasonText(null),
    "Sign-in is unavailable on this Machine.",
  );
});
