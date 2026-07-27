import { assertEquals } from "jsr:@std/assert";
import {
  INSPECT_PRESS_MAX_MS,
  INSPECT_PRESS_MIN_MS,
  isInspectPress,
} from "./codeInspectGesture.ts";

Deno.test("code inspect requires a deliberate short press", () => {
  assertEquals(
    isInspectPress({
      durationMs: INSPECT_PRESS_MIN_MS,
      movementPx: 0,
    }),
    true,
  );
  assertEquals(
    isInspectPress({
      durationMs: INSPECT_PRESS_MIN_MS - 1,
      movementPx: 0,
    }),
    false,
  );
});

Deno.test("code inspect yields to scrolling and native long press", () => {
  assertEquals(
    isInspectPress({
      durationMs: 300,
      movementPx: 11,
    }),
    false,
  );
  assertEquals(
    isInspectPress({
      durationMs: INSPECT_PRESS_MAX_MS + 1,
      movementPx: 0,
    }),
    false,
  );
});
