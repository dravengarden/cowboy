import { assert, assertEquals, assertFalse } from "jsr:@std/assert";
import {
  type AnnouncementMemory,
  legacyRecordsAnnouncement,
} from "./legacyRecordsNotice.ts";

function memory(): AnnouncementMemory & { store: Map<string, string> } {
  const store = new Map<string, string>();
  return {
    store,
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => {
      store.set(key, value);
    },
    removeItem: (key) => {
      store.delete(key);
    },
  };
}

const KEYS = ["cowboy:sync:queue:session-a", "cowboy:sync:service:title"];

Deno.test("a device is told once, and never again", () => {
  const device = memory();
  assert(legacyRecordsAnnouncement(KEYS, device).announce);
  for (let reload = 0; reload < 20; reload += 1) {
    const again = legacyRecordsAnnouncement(KEYS, device);
    assertFalse(again.announce);
    assertEquals(again.reason, "already-announced");
  }
});

// The regression this file exists for: gating used to hash the retained SET, so
// any drift in it re-announced. An iPad PWA reloads whenever iOS evicts the web
// view, and a morning of reloads produced ~20 identical warnings.
Deno.test("a changed retained set does not re-announce", () => {
  const device = memory();
  assert(
    legacyRecordsAnnouncement(["cowboy:sync:queue:session-a"], device).announce,
  );
  const grown = legacyRecordsAnnouncement(
    ["cowboy:sync:queue:session-a", "cowboy:sync:queue:session-b"],
    device,
  );
  assertFalse(grown.announce);
  assertEquals(grown.reason, "already-announced");
  assertEquals(grown.count, 2);
});

Deno.test("an emptied set forgets, so a genuinely new one speaks once", () => {
  const device = memory();
  assert(legacyRecordsAnnouncement(KEYS, device).announce);
  const empty = legacyRecordsAnnouncement([], device);
  assertFalse(empty.announce);
  assertEquals(empty.reason, "empty");
  assertEquals(device.store.size, 0);
  assert(legacyRecordsAnnouncement(KEYS, device).announce);
});

// Deliberate reversal of the previous behaviour: a warning the device cannot
// remember making is a warning it makes on EVERY load. The records stay listed
// in Settings → Info either way, so silence is the lesser harm — and the
// decision is reported to telemetry so it is never invisible.
Deno.test("a device that cannot remember is not nagged", () => {
  assertEquals(legacyRecordsAnnouncement(KEYS, null), {
    announce: false,
    reason: "unrecordable",
    count: 2,
  });
  const blocked: AnnouncementMemory = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
    removeItem: () => {},
  };
  assertEquals(legacyRecordsAnnouncement(KEYS, blocked).reason, "unrecordable");
  // A store that accepts the write and silently drops it — iOS at quota.
  const amnesiac: AnnouncementMemory = {
    getItem: () => null,
    setItem: () => {},
    removeItem: () => {},
  };
  assertEquals(
    legacyRecordsAnnouncement(KEYS, amnesiac).reason,
    "unrecordable",
  );
});

Deno.test("every load reports its decision, warned or not", async () => {
  const store = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
  const notice = store.slice(store.indexOf("syncDatabase.legacyRecords()"));
  assert(notice.includes('reportClientLog("info", "legacy_records_notice"'));
  assert(notice.includes("reason: announcement.reason"));
  assert(notice.includes("retained: announcement.count"));
  assert(notice.includes("if (announcement.announce) {"));
});
