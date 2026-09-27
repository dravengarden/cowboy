import { assert, assertEquals } from "jsr:@std/assert";
import type { ClientSnapshot, Mutation } from "@cowboy/state-sync";
import { replicatedStore } from "@cowboy/state-sync";
import { createIdbPersistenceOwner } from "../../components/state-sync-idb/index.ts";
import { FakeIndexedDb } from "./idbPersistence.fixture.ts";
import { createSyncShutdown } from "./syncShutdown.ts";

/** Every durable write must adopt its IndexedDB outbox baseline first. The
 *  outbox rejects a `save` issued while its own read is still in flight (or has
 *  not started) with `outbox_loading`, and a send tapped moments after a reload
 *  then fails instead of being saved — the draft ▶ path shipped that way. */
Deno.test("no durable write runs before its outbox baseline is adopted", async () => {
  const source = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
  const starts = [...source.matchAll(/^(?:export )?(?:async )?function \w+/gm)]
    .map((match) => match.index ?? 0);
  assert(starts.length > 0);
  let checked = 0;
  for (
    let at = source.indexOf(".mutateDurably(");
    at >= 0;
    at = source.indexOf(".mutateDurably(", at + 1)
  ) {
    const body = source.slice(starts.filter((start) => start < at).pop() ?? 0, at);
    assert(
      /await (?:durableQueue|restoreQueue)\(/.test(body) || body.includes(".hydrate()"),
      `a durable write near offset ${at} does not adopt its outbox baseline first`,
    );
    checked += 1;
  }
  assert(checked >= 9, `expected every durable write to be covered, saw ${checked}`);

  // Discarding a row is durable too: it removes an outbox mutation.
  const discard = source.slice(
    source.indexOf("async function discardQueueMutationDurably("),
    source.indexOf("function pendingNamed("),
  );
  assert(discard.includes("await durableQueue(sessionId)"));
});

/** The restore is memoized per session, so the write barrier above waits for
 *  exactly the work lazy creation started instead of racing a second one. */
Deno.test("queue restore keeps held decisions ahead of the outbox replay", async () => {
  const source = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
  const restore = source.slice(
    source.indexOf("function restoreQueue("),
    source.indexOf("async function durableQueue("),
  );
  assert(restore.indexOf("restoreHeld(sessionId)") < restore.indexOf("store.hydrate()"));
  assert(restore.indexOf("forgetSettledHeld(sessionId, held)") < restore.indexOf("store.resend()"));
  assert(restore.includes("qRestores.set(sessionId, restore)"));
});

/** The barrier `durableQueue` applies, exercised against the real outbox: the
 *  same tap that the component rejects mid-read is saved once the store adopted
 *  its baseline first. (`idbOutbox.test.ts` owns the rejecting half.) */
Deno.test("a send tapped while the queue is still restoring is saved, not rejected", async () => {
  const factory = new FakeIndexedDb();
  factory.autoTransactions = true;
  factory.data.set("queue", {
    base: { version: 0, value: 0 },
    pending: [{ id: "a", client: "fixture", name: "add", args: 1 }],
  });
  const owner = createIdbPersistenceOwner({ factory });
  const sent: Mutation[] = [];
  const store = replicatedStore({
    clientId: "fixture",
    initial: 0,
    mutators: { add: (value: number, amount: number): number => value + amount },
    send: (m) => sent.push(m),
    local: owner.outbox<number>("queue"),
  });

  // What every durable queue write now does before it saves.
  const restore = store.hydrate();
  await restore;
  await store.mutateDurably("add", 1, "b");

  assertEquals(sent.map((m) => m.id), ["b"]);
  assertEquals(store.pending().map((m) => m.id), ["a", "b"]);
  await createSyncShutdown(owner)([store]);
  const stored = factory.data.get("queue") as ClientSnapshot<number>;
  assertEquals(stored.pending.map((m) => m.id), ["a", "b"]);
});
