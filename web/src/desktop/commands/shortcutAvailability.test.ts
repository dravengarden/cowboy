import { assertEquals } from "jsr:@std/assert";
import {
  sequentialShortcutAvailability,
  shortcutAvailability,
} from "./shortcutAvailability.ts";

Deno.test("context shortcuts distinguish inactive, available, and active", () => {
  assertEquals(shortcutAvailability(false), "inactive");
  assertEquals(shortcutAvailability(true), "available");
  assertEquals(shortcutAvailability(true, true), "active");
  assertEquals(shortcutAvailability(false, true), "inactive");
});

Deno.test("sequential shortcut prefix and continuation expose truthful states", () => {
  assertEquals(
    sequentialShortcutAvailability({ scopeAvailable: false, armed: false, prefix: true }),
    "inactive",
  );
  assertEquals(
    sequentialShortcutAvailability({ scopeAvailable: true, armed: false, prefix: true }),
    "available",
  );
  assertEquals(
    sequentialShortcutAvailability({ scopeAvailable: true, armed: false, prefix: false }),
    "inactive",
  );
  assertEquals(
    sequentialShortcutAvailability({ scopeAvailable: true, armed: true, prefix: true }),
    "active",
  );
  assertEquals(
    sequentialShortcutAvailability({ scopeAvailable: true, armed: true, prefix: false }),
    "available",
  );
});
