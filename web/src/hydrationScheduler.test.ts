import { assertEquals } from "jsr:@std/assert";
import { createPrefetchRunner, prefetchCandidates } from "./hydrationScheduler.ts";
import type { Status } from "./protocol.ts";

function session(id: string, status: Status = "running"): { id: string; status: Status } {
  return { id, status };
}

Deno.test("busy sessions come first, then the MRU newest first, without the active or hydrated ones", () => {
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

Deno.test("candidates are capped and never name a session the Hub no longer lists", () => {
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

Deno.test("the runner bounds concurrency, dedupes in-flight work and preempts on cancel", async () => {
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
