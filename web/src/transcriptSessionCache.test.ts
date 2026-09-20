import { assertEquals } from "jsr:@std/assert";
import {
  retainTranscriptSessionCache,
  touchTranscriptSessionCache,
  TRANSCRIPT_SESSION_CACHE_LIMIT,
} from "./transcriptSessionCache";
import { prefetchCandidates } from "./hydrationScheduler";

Deno.test("transcript session cache keeps the current session and evicts LRU history", () => {
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

Deno.test("transcript session cache drops deleted sessions", () => {
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
Deno.test("prefetch rounds never evict the opened transcript", () => {
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

Deno.test("pinning evicts the next coldest session instead", () => {
  const touched = touchTranscriptSessionCache(["a", "b", "c"], "d", 3, "a");
  assertEquals(touched.order, ["a", "c", "d"]);
  assertEquals(touched.evicted, ["b"]);
});
