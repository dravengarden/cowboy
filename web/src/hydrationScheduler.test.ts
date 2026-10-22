import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { createPrefetchRunner, prefetchCandidates } from "./hydrationScheduler.ts";
import type { Status } from "./protocol.ts";

function session(id: string, status: Status = "running"): { id: string; status: Status } {
  return { id, status };
}

test("busy sessions come first, then the MRU newest first, without the active or hydrated ones", () => {
  assertEquals(
    prefetchCandidates({
      sessions: [
        session("a"),
        session("b", "busy"),
        session("c"),
        session("d", "busy"),
        session("e"),
      ],
      activeId: "d",
      recent: ["c", "a", "e", "d"],
      hydrated: new Set(["e"]),
      limit: 5,
    }),
    ["b", "a", "c"],
  );
});

test("candidates are capped and never name a session the Hub no longer lists", () => {
  assertEquals(
    prefetchCandidates({
      sessions: [session("a"), session("b"), session("c")],
      activeId: undefined,
      recent: ["gone", "a", "b", "c"],
      hydrated: new Set(),
      limit: 2,
    }),
    ["c", "b"],
  );
  assertEquals(
    prefetchCandidates({
      sessions: [session("a", "busy")],
      activeId: "a",
      recent: ["a"],
      hydrated: new Set(),
      limit: 5,
    }),
    [],
  );
});

test("prefetch only fills free cache slots and never evicts a cached transcript", () => {
  // Eight busy sessions, six slots, one opened: the cache is already full of
  // hydrated transcripts, so no busy session may push another one out.
  const busy = ["a", "b", "c", "d", "e", "f", "g", "h"].map((id) => session(id, "busy"));
  assertEquals(
    prefetchCandidates({
      sessions: busy,
      activeId: "a",
      recent: ["b", "c", "d", "e", "f", "a"],
      hydrated: new Set(["a", "b", "c", "d", "e", "f"]),
      limit: 5,
      capacity: 6,
    }),
    [],
  );
  // Two free slots, one of them held for the not-yet-touched opened session.
  assertEquals(
    prefetchCandidates({
      sessions: busy,
      activeId: "a",
      recent: ["b", "c", "d", "e"],
      hydrated: new Set(["b", "c", "d", "e"]),
      limit: 5,
      capacity: 6,
    }),
    ["f"],
  );
  // A cached entry whose fetch failed retries in its own slot.
  assertEquals(
    prefetchCandidates({
      sessions: busy,
      activeId: "a",
      recent: ["b", "c", "d", "e", "f", "a"],
      hydrated: new Set(["a", "b", "c", "d", "e"]),
      limit: 5,
      capacity: 6,
    }),
    ["f"],
  );
});

test("the runner bounds concurrency, dedupes in-flight work and preempts on cancel", async () => {
  const started: string[] = [];
  const aborted: string[] = [];
  const resolvers = new Map<string, () => void>();
  const runner = createPrefetchRunner({
    concurrency: 2,
    start: (id) => {
      started.push(id);
      return new Promise<void>((resolve) => resolvers.set(id, resolve));
    },
    abort: (id) => aborted.push(id),
  });

  runner.schedule(["a", "b", "c"]);
  assertEquals(started, ["a", "b"]);
  assertEquals(runner.queued, ["c"]);

  // A re-schedule keeps in-flight fetches and replaces the queue.
  runner.schedule(["b", "d", "a"]);
  assertEquals(started, ["a", "b"]);
  assertEquals(runner.queued, ["d"]);

  resolvers.get("a")!();
  await Promise.resolve();
  await Promise.resolve();
  assertEquals(started, ["a", "b", "d"]);
  assertEquals(runner.inFlight.slice().sort(), ["b", "d"]);

  runner.cancel("d");
  assertEquals(aborted, ["b"], "the excepted fetch keeps running");
  assertEquals(runner.queued, []);
  runner.cancel();
  // Aborting an already aborted fetch again is harmless.
  assertEquals(aborted, ["b", "b", "d"]);
  resolvers.get("b")!();
  resolvers.get("d")!();
  await Promise.resolve();
  await Promise.resolve();
  assertEquals(started, ["a", "b", "d"], "a cancelled generation must not pump");

  runner.schedule(["e"]);
  assertEquals(started, ["a", "b", "d", "e"]);
});
