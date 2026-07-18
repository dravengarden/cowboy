import { assertEquals } from "jsr:@std/assert";
import { createServiceWorkerUpdateCheck } from "./serviceWorkerUpdates.ts";

function deferred(): {
  promise: Promise<void>;
  resolve: () => void;
  reject: () => void;
} {
  let resolve!: () => void;
  let reject!: () => void;
  const promise = new Promise<void>((ok, fail) => {
    resolve = ok;
    reject = fail;
  });
  return { promise, resolve, reject };
}

Deno.test("service worker update checks coalesce resume event bursts", async () => {
  let calls = 0;
  let now = 10_000;
  const pending = deferred();
  const check = createServiceWorkerUpdateCheck(() => {
    calls++;
    return pending.promise;
  }, () => now);

  check();
  check();
  assertEquals(calls, 1);
  pending.resolve();
  await pending.promise;
  await Promise.resolve();

  now += 4_999;
  check();
  assertEquals(calls, 1);
  now++;
  check();
  assertEquals(calls, 2);
});

Deno.test("failed update checks retry immediately after network recovery", async () => {
  let calls = 0;
  const failed = deferred();
  const check = createServiceWorkerUpdateCheck(() => {
    calls++;
    return calls === 1 ? failed.promise : Promise.resolve();
  }, () => 10_000);

  check();
  failed.reject();
  await failed.promise.catch(() => {});
  await Promise.resolve();
  check();
  assertEquals(calls, 2);
});
