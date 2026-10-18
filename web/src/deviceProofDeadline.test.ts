import { test } from "bun:test";
import { assertEquals, assertRejects } from "@std/assert";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(
  new URL("../device-proof.js", import.meta.url),
  "utf8",
);

// Run the shipped page/worker script in its own realm. Identity and signing
// are fixtures; the separate HTTPS browser gate verifies real WebCrypto.
function fixture(fetch: typeof globalThis.fetch) {
  class Key {}
  const keys = { privateKey: new Key(), publicKey: new Key() };
  const timers = new Map<number, () => void>();
  let nextTimer = 0;
  const scope: any = {
    fetch,
    URL,
    Request,
    Headers,
    AbortController,
    TextEncoder,
    Uint8Array,
    CryptoKey: Key,
    btoa,
    location: { origin: "https://fixture.invalid" },
    setTimeout: (callback: () => void, ms: number) => {
      assertEquals(ms, 8_000);
      timers.set(++nextTimer, callback);
      return nextTimer;
    },
    clearTimeout: (id: number) => timers.delete(id),
    indexedDB: {
      open: () => {
        const request: any = {};
        queueMicrotask(() => {
          request.result = {
            close() {},
            transaction: () => {
              const tx: any = {
                objectStore: () => ({ get: () => ({ result: keys }) }),
              };
              queueMicrotask(() => tx.oncomplete());
              return tx;
            },
          };
          request.onsuccess();
        });
        return request;
      },
    },
    crypto: {
      getRandomValues: (bytes: Uint8Array) => bytes,
      subtle: {
        exportKey: async () => new Uint8Array(65),
        sign: async () => new Uint8Array(64),
      },
    },
  };
  runInNewContext(source, scope);
  return { device: scope.CowboyDeviceProof, timers };
}

for (const stalled of ["headers", "body"]) {
  test(`device challenge aborts stalled ${stalled} and permits a fresh signed retry`, async () => {
    let requests = 0;
    let aborted = false;
    const { device, timers } = fixture(
      (async (_input, init) => {
        requests++;
        if (requests > 1) {
          return Response.json({
            epoch: "current",
            server_time_ms: Date.now(),
          });
        }
        const pending = new Promise((_, reject) =>
          init!.signal!.addEventListener("abort", () => {
            aborted = true;
            reject(new Error("aborted"));
          })
        );
        return stalled === "headers"
          ? pending
          : { ok: true, json: () => pending };
      }) as typeof fetch,
    );
    const first = device.proof("/ws");
    const peer = device.proof("/api/private");
    const failures = Promise.all([
      assertRejects(() => first),
      assertRejects(() => peer),
    ]);
    assertEquals(requests, 1);
    for (const timeout of timers.values()) timeout();
    await failures;
    assertEquals(aborted, true);
    assertEquals(timers.size, 0);
    const result = await device.proof("/ws");
    assertEquals(typeof result, "string");
    assertEquals(requests, 2);
    await device.proof("/ws");
    assertEquals(requests, 2); // A successful epoch remains reusable.
    assertEquals(timers.size, 0);
  });
}
