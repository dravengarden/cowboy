import { assertEquals } from "jsr:@std/assert";
import { IdbPersistenceError } from "@cowboy/state-sync-idb";
import {
  actionErrorMessage,
  LOCAL_STORAGE_FAILURE_MESSAGE,
  NETWORK_FAILURE_MESSAGE,
} from "./actionErrorMessage";

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

Deno.test("a storage refusal reads as a retryable local failure, keeping its code", () => {
  assertEquals(
    actionErrorMessage(new IdbPersistenceError("outbox_loading"), "fallback"),
    `${LOCAL_STORAGE_FAILURE_MESSAGE} (outbox_loading)`,
  );
});
