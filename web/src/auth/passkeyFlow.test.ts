import { assertEquals, assertRejects } from "jsr:@std/assert";
import { AuthApiError } from "./authApi.ts";
import {
  externalPasskeyEventsUrl,
  externalPasskeyUrl,
  passkeyErrorMessage,
  passkeyFlowCancelled,
  reconcileExternalPasskeyAfterBrowserClose,
  reconcileExternalPasskeyAfterResume,
  waitForExternalPasskeyEvent,
} from "./passkeyFlow.ts";
import { NATIVE_APP_RESUMED_EVENT } from "../openExternal.ts";

class SilentWebSocket extends EventTarget {
  closeCalls = 0;

  close(): void {
    this.closeCalls += 1;
  }

  send(): void {}
}

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

Deno.test("external Passkey event wait has a hard deadline when iOS suspends the socket", async () => {
  const socket = new SilentWebSocket();
  await assertRejects(
    () =>
      waitForExternalPasskeyEvent(
        "a".repeat(64),
        "v".repeat(64),
        new AbortController().signal,
        1,
        {
          createSocket: () => socket as unknown as WebSocket,
          origin: "https://cowboy.example",
        },
      ),
    AuthApiError,
    "Passkey setup timed out",
  );
  assertEquals(socket.closeCalls, 1);
});

Deno.test("native foreground resume wakes a suspended Passkey event wait", async () => {
  const socket = new SilentWebSocket();
  const waiting = waitForExternalPasskeyEvent(
    "a".repeat(64),
    "v".repeat(64),
    new AbortController().signal,
    10_000,
    {
      createSocket: () => socket as unknown as WebSocket,
      origin: "https://cowboy.example",
    },
  );
  globalThis.dispatchEvent(new Event(NATIVE_APP_RESUMED_EVENT));
  assertEquals(await waiting, "initiator-resumed");
  assertEquals(socket.closeCalls, 1);
});

Deno.test("returning focus from an in-app Safari sheet wakes Passkey reconciliation", async () => {
  const socket = new SilentWebSocket();
  const waiting = waitForExternalPasskeyEvent(
    "a".repeat(64),
    "v".repeat(64),
    new AbortController().signal,
    10_000,
    {
      createSocket: () => socket as unknown as WebSocket,
      origin: "https://cowboy.example",
    },
  );
  globalThis.dispatchEvent(new Event("focus"));
  assertEquals(await waiting, "initiator-resumed");
  assertEquals(socket.closeCalls, 1);
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

Deno.test("foreground resume probes completion without cancelling an active Passkey", async () => {
  let finalizeCalls = 0;
  let failCalls = 0;
  const result = await reconcileExternalPasskeyAfterResume(
    "a".repeat(64),
    "v".repeat(64),
    new AbortController().signal,
    {
      finalize: () => {
        finalizeCalls += 1;
        return Promise.resolve({ status: "pending" });
      },
      fail: () => {
        failCalls += 1;
        return Promise.resolve({});
      },
      wait: () => Promise.resolve(),
    },
  );
  assertEquals(result.status, "pending");
  assertEquals(finalizeCalls, 10);
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
