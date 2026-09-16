import { assert, assertEquals } from "jsr:@std/assert";
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

const lockSource = await Deno.readTextFile(
  new URL("./PasskeyReauthLock.tsx", import.meta.url),
);

// A dismissed prompt used to leave this card silent, which is indistinguishable
// from a card that did nothing — and the common cause is not a dismissal at
// all: a password manager answered the prompt holding no Passkey for this site.
Deno.test("a dismissed unlock prompt explains itself instead of going quiet", () => {
  assert(lockSource.includes("if (passkeyFlowCancelled(err)) {"));
  assert(lockSource.includes("setHint("));
  assert(lockSource.includes("pick that app in the prompt"));
  assert(lockSource.includes("add a second Passkey in Settings"));
  // Quiet, not alarming: a dismissal is a normal outcome.
  assert(
    lockSource.includes(
      '{!error && hint && <Alert severity="info">{hint}</Alert>}',
    ),
  );
  // A new attempt starts from a clean slate.
  assert(lockSource.includes("setHint(null);"));
  assert(
    lockSource.includes(
      'passkeyErrorMessage(err, "Passkey verification failed", "verify")',
    ),
  );
});
