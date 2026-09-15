import { assert, assertEquals, assertFalse } from "jsr:@std/assert";
import {
  type AnnouncementMemory,
  legacyRecordsFingerprint,
  shouldAnnounceLegacyRecords,
} from "./legacyRecordsNotice.ts";

function memory(): AnnouncementMemory & { store: Map<string, string> } {
  const store = new Map<string, string>();
  return {
    store,
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => {
      store.set(key, value);
    },
  };
}

Deno.test("an unchanged retained set is announced once per device", () => {
  const device = memory();
  const keys = ["cowboy:sync:queue:session-a", "cowboy:sync:service:title"];
  assert(shouldAnnounceLegacyRecords(keys, device));
  assertFalse(shouldAnnounceLegacyRecords(keys, device));
  assertFalse(shouldAnnounceLegacyRecords([...keys].reverse(), device));
});

Deno.test("a changed retained set is announced again", () => {
  const device = memory();
  assert(shouldAnnounceLegacyRecords(["cowboy:sync:queue:session-a"], device));
  assert(shouldAnnounceLegacyRecords(
    ["cowboy:sync:queue:session-a", "cowboy:sync:queue:session-b"],
    device,
  ));
  assertFalse(shouldAnnounceLegacyRecords(
    ["cowboy:sync:queue:session-b", "cowboy:sync:queue:session-a"],
    device,
  ));
});

Deno.test("an empty retained set is never announced", () => {
  const device = memory();
  assertFalse(shouldAnnounceLegacyRecords([], device));
  assertEquals(device.store.size, 0);
});

Deno.test("without durable memory the notice still reaches the reader", () => {
  const keys = ["cowboy:sync:queue:session-a"];
  assert(shouldAnnounceLegacyRecords(keys, null));
  assert(shouldAnnounceLegacyRecords(keys, null));
  const throwing: AnnouncementMemory = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  };
  assert(shouldAnnounceLegacyRecords(keys, throwing));
});

Deno.test("the fingerprint is bounded and distinguishes sets", () => {
  const many = Array.from(
    { length: 4096 },
    (_unused, index) => `cowboy:sync:queue:session-${index}`,
  );
  const fingerprint = legacyRecordsFingerprint(many);
  assert(fingerprint.length <= 16, fingerprint);
  assertEquals(fingerprint, legacyRecordsFingerprint([...many].reverse()));
  assert(
    legacyRecordsFingerprint(["cowboy:sync:a"]) !==
      legacyRecordsFingerprint(["cowboy:sync:b"]),
  );
});
