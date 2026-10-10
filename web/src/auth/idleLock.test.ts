import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  ADMIN_PASSKEY_IDLE_MS,
  idleLockShouldEngage,
  noteActivity,
} from "./idleLock.ts";

const TEST_IDLE_MS = 15 * 60 * 1_000;

test("idle lock waits for the full idle window", () => {
  assertEquals(
    idleLockShouldEngage({
      eligible: true,
      alreadyLocked: false,
      nowMs: TEST_IDLE_MS,
      lastActiveMs: 0,
      idleAfterMs: TEST_IDLE_MS,
    }),
    true,
  );
  assertEquals(
    idleLockShouldEngage({
      eligible: true,
      alreadyLocked: false,
      nowMs: TEST_IDLE_MS - 1,
      lastActiveMs: 0,
      idleAfterMs: TEST_IDLE_MS,
    }),
    false,
  );
});

test("idle lock never engages without a passkey or when turned off", () => {
  assertEquals(
    idleLockShouldEngage({
      eligible: false,
      alreadyLocked: false,
      nowMs: TEST_IDLE_MS + 1,
      lastActiveMs: 0,
      idleAfterMs: TEST_IDLE_MS,
    }),
    false,
  );
});

test("activity while unlocked resets the idle clock", () => {
  assertEquals(noteActivity({ alreadyLocked: false, nowMs: 42 }), 42);
  assertEquals(noteActivity({ alreadyLocked: true, nowMs: 42 }), null);
});

test("admin idle window is five minutes", () => {
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
