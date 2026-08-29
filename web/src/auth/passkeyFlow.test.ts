import { assertEquals } from "jsr:@std/assert";
import { AuthApiError } from "./authApi.ts";
import {
  externalPasskeyUrl,
  passkeyErrorMessage,
} from "./passkeyFlow.ts";

Deno.test("external Passkey URL carries only the opaque transaction in a fragment", () => {
  const transactionId = "a".repeat(64);
  const url = new URL(
    externalPasskeyUrl("https://cowboy.example", transactionId),
  );
  assertEquals(url.origin, "https://cowboy.example");
  assertEquals(url.pathname, "/passkey.html");
  assertEquals(url.search, "");
  assertEquals(
    new URLSearchParams(url.hash.slice(1)).get("transaction"),
    transactionId,
  );
  assertEquals(url.href.includes("verifier"), false);
});

Deno.test("Passkey failures preserve server errors and explain browser cancellation", () => {
  assertEquals(
    passkeyErrorMessage(new AuthApiError("Passkey setup expired", 400), "fallback"),
    "Passkey setup expired",
  );
  assertEquals(
    passkeyErrorMessage(new DOMException("cancelled", "NotAllowedError"), "fallback"),
    "Passkey verification was cancelled or timed out.",
  );
  assertEquals(passkeyErrorMessage(new Error("private"), "fallback"), "fallback");
});
