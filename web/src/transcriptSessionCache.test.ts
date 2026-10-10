import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  CREATED_SESSION_LISTING_GRACE_MS,
  listedOrJustCreatedSessions,
  retainTranscriptSessionCache,
  touchTranscriptSessionCache,
  TRANSCRIPT_SESSION_CACHE_LIMIT,
} from "./transcriptSessionCache";
import { prefetchCandidates } from "./hydrationScheduler";

test("transcript session cache keeps the current session and evicts LRU history", () => {
  let order: string[] = [];
  for (const id of ["a", "b", "c", "d"]) {
    order = touchTranscriptSessionCache(order, id, 3).order;
  }
  assertEquals(order, ["b", "c", "d"]);

  const revisited = touchTranscriptSessionCache(order, "b", 3);
  assertEquals(revisited.order, ["c", "d", "b"]);
  assertEquals(revisited.evicted, []);

  const opened = touchTranscriptSessionCache(revisited.order, "e", 3);
  assertEquals(opened.order, ["d", "b", "e"]);
  assertEquals(opened.evicted, ["c"]);
});

test("transcript session cache drops deleted sessions", () => {
  const retained = retainTranscriptSessionCache(
    ["a", "b", "c"],
    new Set(["a", "c"]),
  );
  assertEquals(retained.order, ["a", "c"]);
  assertEquals(retained.evicted, ["b"]);
});

// Regression: background prefetch rounds kept touching new sessions, walking
// the OPENED transcript to the cold end until its own prefetches evicted it.
// The evicted active session dropped `hydrated`, prefetch skips the active id,
// and every later snapshot/event was discarded as uncached — the transcript sat
// on its loading skeleton until the user reopened it.
test("prefetch rounds never evict the opened transcript", () => {
  const opened = "active";
  let order = touchTranscriptSessionCache(
    [],
    opened,
    TRANSCRIPT_SESSION_CACHE_LIMIT,
    opened,
  ).order;
  const hydrated = new Set<string>([opened]);
  const sessions = [{ id: opened, status: "busy" as const }];
  for (let round = 0; round < 6; round += 1) {
    // Each round a fresh batch turns busy, exactly as the daemon's `sessions`
    // broadcasts drive `schedulePrefetch`.
    for (let n = 0; n < 3; n += 1) {
      sessions.push({
        id: `s${String(round)}-${String(n)}`,
        status: "busy" as const,
      });
    }
    for (
      const id of prefetchCandidates({
        sessions,
        activeId: opened,
        recent: order,
        hydrated,
        limit: TRANSCRIPT_SESSION_CACHE_LIMIT - 1,
      })
    ) {
      const update = touchTranscriptSessionCache(
        order,
        id,
        TRANSCRIPT_SESSION_CACHE_LIMIT,
        opened,
      );
      order = update.order;
      for (const victim of update.evicted) hydrated.delete(victim);
      hydrated.add(id);
    }
    assertEquals(order.includes(opened), true);
    assertEquals(hydrated.has(opened), true);
  }
  assertEquals(order.length, TRANSCRIPT_SESSION_CACHE_LIMIT);
});

test("pinning evicts the next coldest session instead", () => {
  const touched = touchTranscriptSessionCache(["a", "b", "c"], "d", 3, "a");
  assertEquals(touched.order, ["a", "c", "d"]);
  assertEquals(touched.evicted, ["b"]);
});

test("a list produced before a creation does not evict the session just opened", () => {
  const created = new Map([["new", 1_000]]);
  // The stale frame arrives after the client opened the created session.
  const stale = listedOrJustCreatedSessions(new Set(["a"]), created, 1_050);
  assertEquals(
    retainTranscriptSessionCache(["a", "new"], stale).evicted,
    [],
  );
  assertEquals([...created.keys()], ["new"]);

  // The authoritative list names it; later absences are real deletions.
  listedOrJustCreatedSessions(new Set(["a", "new"]), created, 1_100);
  assertEquals(created.size, 0);
  const deleted = listedOrJustCreatedSessions(new Set(["a"]), created, 1_200);
  assertEquals(
    retainTranscriptSessionCache(["a", "new"], deleted).evicted,
    ["new"],
  );
});

test("a created session that no list ever names stops being protected", () => {
  const created = new Map([["new", 1_000]]);
  const valid = listedOrJustCreatedSessions(
    new Set(["a"]),
    created,
    1_000 + CREATED_SESSION_LISTING_GRACE_MS,
  );
  assertEquals([...valid], ["a"]);
  assertEquals(created.size, 0);
});

test("the store prunes against created sessions and recovers an opened skeleton", async () => {
  const store = await readFile(new URL("./store.ts", import.meta.url), "utf8");
  const listHandler = store.slice(
    store.indexOf('case "sessions": {'),
    store.indexOf('case "machines": {'),
  );
  // Every pruner reads the widened set, so none can run on the raw list.
  assertEquals(listHandler.includes("new Set(msg.sessions.map((s) => s.id)),\n        unlistedCreatedSessions,"), true);
  assertEquals(listHandler.includes("retainTranscriptSessions(validSessions);"), true);
  assertEquals(listHandler.includes("void hydrateSession(openedSessionId);"), true);
  const created = store.slice(store.indexOf("export function markSessionHydrated("));
  assertEquals(created.slice(0, 600).includes("unlistedCreatedSessions.set(id, Date.now());"), true);
});
