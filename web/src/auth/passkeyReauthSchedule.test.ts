import { assertEquals } from "jsr:@std/assert";
import {
  passkeyReauthDue,
  passkeyReauthTimerDelay,
} from "./passkeyReauthSchedule.ts";

Deno.test("Passkey lock is due only for an eligible browser", () => {
  assertEquals(passkeyReauthDue(false, true, null, 100), false);
  assertEquals(passkeyReauthDue(true, true, null, 100), true);
  assertEquals(passkeyReauthDue(true, false, 99, 100), true);
  assertEquals(passkeyReauthDue(true, false, 101, 100), false);
});

Deno.test("Passkey lock schedules the exact future deadline", () => {
  assertEquals(passkeyReauthTimerDelay(true, false, 250, 100), 150);
  assertEquals(passkeyReauthTimerDelay(true, false, 100, 100), null);
  assertEquals(passkeyReauthTimerDelay(true, true, 250, 100), null);
  assertEquals(passkeyReauthTimerDelay(false, false, 250, 100), null);
});
