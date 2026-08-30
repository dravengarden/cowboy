import { assertEquals, assertRejects } from "jsr:@std/assert";
import { AuthApiError } from "./authApi.ts";
import {
  externalPasskeyEventsUrl,
  externalPasskeyUrl,
  passkeyErrorMessage,
  passkeyFlowCancelled,
  reconcileExternalPasskeyAfterBrowserClose,
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

Deno.test("external Passkey events use a same-origin WebSocket without secrets", () => {
  const url = new URL(externalPasskeyEventsUrl("https://cowboy.example/app"));
  assertEquals(url.protocol, "wss:");
  assertEquals(url.host, "cowboy.example");
  assertEquals(url.pathname, "/api/auth/passkeys/external/events");
  assertEquals(url.search, "");
  assertEquals(url.hash, "");
});

Deno.test("Passkey failures preserve server errors and explain browser cancellation", () => {
  assertEquals(
    passkeyErrorMessage(
      new AuthApiError("Passkey setup expired", 400),
      "fallback",
    ),
    "Passkey setup expired",
  );
  assertEquals(
    passkeyErrorMessage(
      new DOMException("cancelled", "NotAllowedError"),
      "fallback",
    ),
    "Passkey verification was cancelled or timed out.",
  );
  assertEquals(
    passkeyErrorMessage(
      new DOMException("cancelled", "AbortError"),
      "fallback",
    ),
    "Passkey verification was cancelled or timed out.",
  );
  assertEquals(
    passkeyErrorMessage(new Error("private"), "fallback"),
    "fallback",
  );
});

Deno.test("Passkey cancellation is a normal user outcome", () => {
  assertEquals(
    passkeyFlowCancelled(new DOMException("cancelled", "NotAllowedError")),
    true,
  );
  assertEquals(
    passkeyFlowCancelled(new DOMException("cancelled", "AbortError")),
    true,
  );
  assertEquals(
    passkeyFlowCancelled(new AuthApiError("Passkey setup was cancelled", 400)),
    true,
  );
  assertEquals(
    passkeyFlowCancelled(new AuthApiError("Passkey setup expired", 400)),
    false,
  );
});

Deno.test("closing the native browser finalizes a completed Passkey after foreground resume", async () => {
  let finalizeCalls = 0;
  let failCalls = 0;
  const result = await reconcileExternalPasskeyAfterBrowserClose(
    "a".repeat(64),
    "v".repeat(64),
    new AbortController().signal,
    {
      finalize: () => {
        finalizeCalls += 1;
        return Promise.resolve(finalizeCalls === 1
          ? { status: "pending" as const }
          : {
            status: "complete" as const,
            passkey: {
              id: "pk-1",
              nickname: "iPhone",
              created_at_ms: 1,
            },
          });
      },
      fail: () => {
        failCalls += 1;
        return Promise.resolve({});
      },
      wait: () => Promise.resolve(),
    },
  );
  assertEquals(result.status, "complete");
  assertEquals(finalizeCalls, 2);
  assertEquals(failCalls, 0);
});

Deno.test("closing an unfinished native Passkey request cancels it after bounded reconciliation", async () => {
  let failCalls = 0;
  await assertRejects(
    () =>
      reconcileExternalPasskeyAfterBrowserClose(
        "a".repeat(64),
        "v".repeat(64),
        new AbortController().signal,
        {
          finalize: () => Promise.resolve({ status: "pending" }),
          fail: () => {
            failCalls += 1;
            return Promise.resolve({});
          },
          wait: () => Promise.resolve(),
        },
      ),
    DOMException,
    "Cancelled",
  );
  assertEquals(failCalls, 1);
});
