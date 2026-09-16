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

const sheetSource = await Deno.readTextFile(
  new URL("./ProductRecentAuthSheet.tsx", import.meta.url),
);

Deno.test("verification outcomes render beside the button that caused them", () => {
  // The body scrolls. An alert inserted above the method tabs lands off-screen
  // on a phone, which is what "I tapped Verify and nothing happened" looks like.
  const actionArea = sheetSource.slice(sheetSource.indexOf("{error && <Alert"));
  assert(actionArea.indexOf("<Tabs") > 0);
  assert(
    actionArea.indexOf("Verify with Passkey") > actionArea.indexOf("<Tabs"),
  );
  assertEquals(
    sheetSource.indexOf("{error && <Alert") <
      sheetSource.indexOf("Identity verified"),
    false,
  );
  // Timing decides which cancellation wording the reader gets.
  assert(sheetSource.includes("const startedAtMs = Date.now();"));
  assert(sheetSource.includes("passkeyPromptWasUntouched(startedAtMs)"));
  // The header no longer explains a step that has not happened yet; the resume
  // instruction sits with the button it describes.
  assertEquals(sheetSource.includes("Verify now, then $"), false);
  assert(sheetSource.includes("Safari only opens the Passkey prompt from a"));
});

// The sheet's reset effect clears error, notice, and — worse — a verification
// that already succeeded and is waiting for the reader's Continue tap. `me` is
// a fresh object on every session push, so depending on it meant any background
// refresh discarded that result and the tap read as doing nothing.
Deno.test("a background session refresh cannot discard a finished verification", () => {
  const effect = sheetSource.slice(
    sheetSource.indexOf("const resetInputs = useRef("),
    sheetSource.indexOf("}, [open, purpose]);") + "}, [open, purpose]);".length,
  );
  assert(effect.includes("setVerifiedMe(null)"));
  assert(effect.endsWith("}, [open, purpose]);"));
  // The values are read through the ref, so they cannot re-trigger the reset.
  assert(effect.includes("const current = resetInputs.current;"));
  assert(effect.includes("current.me"));
  assertEquals(
    sheetSource.includes(
      "}, [accountMethods, me, open, primaryMethods, purpose]);",
    ),
    false,
  );
});
