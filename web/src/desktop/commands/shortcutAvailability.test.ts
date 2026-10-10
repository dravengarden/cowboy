import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  leaderShortcutAvailability,
  sequentialShortcutAvailability,
  shortcutAvailability,
} from "./shortcutAvailability.ts";

test("context shortcuts distinguish inactive, available, and active", () => {
  assertEquals(shortcutAvailability(false), "inactive");
  assertEquals(shortcutAvailability(true), "available");
  assertEquals(shortcutAvailability(true, true), "active");
  assertEquals(shortcutAvailability(false, true), "inactive");
});

test("sequential shortcut prefix and continuation expose truthful states", () => {
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

test("leader slots are available at rest and lit while armed", () => {
  assertEquals(leaderShortcutAvailability(false, true), "inactive");
  assertEquals(leaderShortcutAvailability(true, false), "available");
  assertEquals(leaderShortcutAvailability(true, true), "active");
});
