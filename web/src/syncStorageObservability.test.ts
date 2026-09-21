import { assert } from "jsr:@std/assert";

const source = await Deno.readTextFile(new URL("./store.ts", import.meta.url));

Deno.test("a synced rename adopts its outbox baseline before the durable write", () => {
  const register = source.indexOf("function registerSync<");
  const mutate = source.indexOf("mutate: (name, args): void => {", register);
  const durable = source.indexOf("store.mutateDurably(name, args)", mutate);
  assert(register >= 0 && mutate > register && durable > mutate);
  // The sidebar is interactive while the outbox read is still in flight, so the
  // same barrier the send path runs must precede this state's first write.
  const barrier = source.slice(mutate, durable);
  assert(barrier.includes("store.hydrate()"));
});

Deno.test("local sync storage failures name the state and the storage code", () => {
  const report = source.indexOf("function reportSyncStorageFailure(");
  assert(report >= 0);
  const body = source.slice(report, source.indexOf("\n}", report));
  assert(body.includes("error instanceof IdbPersistenceError ? error.code : \"\""));
  assert(body.includes("sync_state: syncState"));
  assert(body.includes("reportClientLog("));
  // A code the reader can quote turns an unsaved-change toast into a diagnosis.
  assert(body.includes("` (${code})`"));

  for (const event of ['"sync_durable_write_failed"', '"sync_outbox_hydrate_failed"']) {
    const at = source.indexOf(`reportSyncStorageFailure(${event}`);
    assert(at >= 0, `${event} is reported`);
  }
  // Boot hydration is the only place a fenced service outbox is observable
  // before a user write fails, so its rejections may not stay silent.
  const connectHydrate = source.indexOf("const [restored] = await Promise.all(");
  const connectReport = source.indexOf("sync_outbox_hydrate_failed", connectHydrate);
  assert(connectHydrate >= 0 && connectReport > connectHydrate);
  assert(!source.slice(connectHydrate, connectReport).includes("function "));
});
