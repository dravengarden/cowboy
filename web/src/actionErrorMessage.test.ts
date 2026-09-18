import { assertEquals } from "jsr:@std/assert";
import { actionErrorMessage, NETWORK_FAILURE_MESSAGE } from "./actionErrorMessage";

Deno.test("fetch failures read as connectivity, not engine text", () => {
  for (
    const raw of [
      "Load failed",
      "Failed to fetch",
      "NetworkError when attempting to fetch resource.",
      "The network connection was lost.",
    ]
  ) {
    assertEquals(actionErrorMessage(new TypeError(raw), "fallback"), NETWORK_FAILURE_MESSAGE);
  }
});

Deno.test("other failures keep their message or the fallback", () => {
  assertEquals(
    actionErrorMessage(new TypeError("x is undefined"), "fallback"),
    "x is undefined",
  );
  assertEquals(actionErrorMessage(new Error("Session is busy"), "fallback"), "Session is busy");
  assertEquals(actionErrorMessage(new Error("  "), "fallback"), "fallback");
  assertEquals(actionErrorMessage("nope", "fallback"), "fallback");
});
