import { assertEquals } from "jsr:@std/assert";
import {
  ADMIN_PASSKEY_IDLE_MS,
  idleLockShouldEngage,
  noteActivity,
  PRODUCT_PASSKEY_IDLE_MS,
} from "./idleLock.ts";

Deno.test("idle lock waits for the full idle window", () => {
  assertEquals(
    idleLockShouldEngage({
      eligible: true,
      alreadyLocked: false,
      nowMs: PRODUCT_PASSKEY_IDLE_MS,
      lastActiveMs: 0,
      idleAfterMs: PRODUCT_PASSKEY_IDLE_MS,
    }),
    true,
  );
  assertEquals(
    idleLockShouldEngage({
      eligible: true,
      alreadyLocked: false,
      nowMs: PRODUCT_PASSKEY_IDLE_MS - 1,
      lastActiveMs: 0,
      idleAfterMs: PRODUCT_PASSKEY_IDLE_MS,
    }),
    false,
  );
});

Deno.test("idle lock never engages without a passkey or when turned off", () => {
  assertEquals(
    idleLockShouldEngage({
      eligible: false,
      alreadyLocked: false,
      nowMs: PRODUCT_PASSKEY_IDLE_MS + 1,
      lastActiveMs: 0,
      idleAfterMs: PRODUCT_PASSKEY_IDLE_MS,
    }),
    false,
  );
});

Deno.test("activity while unlocked resets the idle clock", () => {
  assertEquals(noteActivity({ alreadyLocked: false, nowMs: 42 }), 42);
  assertEquals(noteActivity({ alreadyLocked: true, nowMs: 42 }), null);
});

Deno.test("admin idle window is five minutes", () => {
  assertEquals(ADMIN_PASSKEY_IDLE_MS, 5 * 60 * 1_000);
  assertEquals(
    idleLockShouldEngage({
      eligible: true,
      alreadyLocked: false,
      nowMs: ADMIN_PASSKEY_IDLE_MS,
      lastActiveMs: 0,
      idleAfterMs: ADMIN_PASSKEY_IDLE_MS,
    }),
    true,
  );
});
