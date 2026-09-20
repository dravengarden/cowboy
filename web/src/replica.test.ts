import { assert, assertEquals } from "jsr:@std/assert";
import type { ProductCache, ProductCacheScope } from "./productSyncDatabase.ts";
import type { Envelope, SessionMeta } from "./protocol.ts";
import {
  createReplica,
  decodeReplicaDelivery,
  decodeReplicaSessions,
  decodeReplicaTail,
  type ReplicaDatabase,
} from "./replica.ts";

function memoryDatabase(): ReplicaDatabase & { readonly data: Map<string, unknown>; saves: number } {
  const data = new Map<string, unknown>();
  const keyOf = (scope: ProductCacheScope): string =>
    scope.kind === "service" ? `service:${scope.state}` : `session:${scope.session}:${scope.state}`;
  const db = {
    data,
    saves: 0,
    cache<T>(scope: ProductCacheScope): ProductCache<T> {
      const key = keyOf(scope);
      return {
        load: () => Promise.resolve((data.has(key) ? structuredClone(data.get(key)) : null) as T | null),
        save: (value: T) => {
          db.saves += 1;
          data.set(key, structuredClone(value));
          return Promise.resolve();
        },
        discard: () => {
          data.delete(key);
          return Promise.resolve();
        },
      };
    },
    cacheSessions: (state: "tail" | "delivery") =>
      Promise.resolve(
        [...data.keys()]
          .filter((key) => key.startsWith("session:") && key.endsWith(`:${state}`))
          .map((key) => key.slice("session:".length, -(state.length + 1))),
      ),
    discardCaches: () => {
      data.clear();
      return Promise.resolve();
    },
  };
  return db;
}

const meta = (id: string): SessionMeta => ({
  id,
  provider: "codex",
  cwd: "/w",
  title: id,
  status: "running",
});

const event = (seq: number): Envelope => ({
  session_id: "a",
  seq,
  kind: "update",
  update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "x" } },
});

const tick = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

Deno.test("decoders reject malformed cache records instead of painting them", () => {
  assertEquals(decodeReplicaSessions(null), null);
  assertEquals(decodeReplicaSessions({ receivedAt: 1, sessions: [{ id: "x" }] }), null);
  assertEquals(decodeReplicaSessions({ receivedAt: 1, sessions: [meta("a")] })?.sessions.length, 1);
  assertEquals(decodeReplicaTail({ receivedAt: 1, lastSeq: 2, reachedStart: false, events: [event(2), event(1)] }), null);
  const tail = decodeReplicaTail({ receivedAt: 1, lastSeq: 2, reachedStart: true, events: [event(1), event(2)], configOptions: "no" });
  assertEquals(tail?.events.length, 2);
  assertEquals(tail?.configOptions, undefined);
  assertEquals(decodeReplicaDelivery({ held: ["a", 1] }), null);
  assertEquals(decodeReplicaDelivery({ held: ["a"] }), { held: ["a"] });
});

Deno.test("a cached tail carries the validator that makes the next open conditional", () => {
  const base = { receivedAt: 1, lastSeq: 1, reachedStart: true, events: [event(1)] };
  assertEquals(decodeReplicaTail({ ...base, etag: '"bootstrap-v1-abc"' })?.etag, '"bootstrap-v1-abc"');
  // A tail written before this existed simply revalidates unconditionally.
  assertEquals(decodeReplicaTail(base)?.etag, undefined);
  // Never replay a value that could not have come from a response header.
  assertEquals(decodeReplicaTail({ ...base, etag: 42 })?.etag, undefined);
  assertEquals(decodeReplicaTail({ ...base, etag: "" })?.etag, undefined);
  assertEquals(decodeReplicaTail({ ...base, etag: "x".repeat(400) })?.etag, undefined);
});

Deno.test("replica coalesces bursts into one write and lands immediate checkpoints", async () => {
  const db = memoryDatabase();
  let clock = 1000;
  const replica = createReplica(db, { now: () => clock, debounceMs: 1, maxWaitMs: 10 });
  replica.recordSessions([meta("a")]);
  replica.recordSessions([meta("a"), meta("b")]);
  await replica.flush();
  assertEquals(db.saves, 1);
  assertEquals((await replica.loadSessions())?.sessions.map((s) => s.id), ["a", "b"]);
  clock = 2000;
  const session = replica.session("a");
  session.scheduleTail(() => ({ receivedAt: clock, lastSeq: 1, reachedStart: true, events: [event(1)] }));
  session.scheduleTail(
    () => ({ receivedAt: clock, lastSeq: 2, reachedStart: true, events: [event(1), event(2)] }),
    { immediate: true },
  );
  await replica.flush();
  await tick();
  assertEquals(db.saves, 2);
  assertEquals((await session.loadTail())?.lastSeq, 2);
  // A null producer means "nothing to persist" and writes nothing.
  session.scheduleTail(() => null, { immediate: true });
  await replica.flush();
  assertEquals(db.saves, 2);
});

Deno.test("replica retains only listed sessions and forgets everything on discardAll", async () => {
  const db = memoryDatabase();
  const replica = createReplica(db, { debounceMs: 1, maxWaitMs: 5 });
  await replica.session("a").saveDelivery({ held: ["m1"] });
  replica.session("b").scheduleTail(
    () => ({ receivedAt: 1, lastSeq: 1, reachedStart: true, events: [event(1)] }),
    { immediate: true },
  );
  await replica.flush();
  await replica.retainSessions(new Set(["a"]));
  assertEquals([...db.data.keys()].sort(), ["session:a:delivery"]);
  assertEquals((await replica.session("a").loadDelivery())?.held, ["m1"]);
  replica.recordMachines(3, []);
  await replica.discardAll();
  assertEquals(db.data.size, 0);
  // Sealed: later schedules are ignored.
  replica.recordSessions([meta("a")]);
  await replica.flush();
  assertEquals(db.data.size, 0);
});

Deno.test("replica seal drops pending producers without writing", async () => {
  const db = memoryDatabase();
  const replica = createReplica(db, { debounceMs: 50, maxWaitMs: 100 });
  replica.recordSessions([meta("a")]);
  replica.seal();
  await replica.flush();
  await new Promise((resolve) => setTimeout(resolve, 60));
  assert(db.data.size === 0);
});
