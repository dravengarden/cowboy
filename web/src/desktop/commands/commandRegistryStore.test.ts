import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { createCommandRegistryStore } from "./commandRegistryStore.ts";

test("commandRegistryStore coalesces synchronous registrations into one microtask notification", async () => {
  const store = createCommandRegistryStore<{ id: string }>();
  let notifications = 0;
  const unsubscribe = store.subscribe(() => {
    notifications += 1;
  });

  store.map.set("a", { id: "a" });
  store.notify();
  store.map.set("b", { id: "b" });
  store.notify();
  store.map.delete("a");
  store.notify();

  assertEquals(notifications, 0);
  // Synchronous readers still see up-to-date state before the microtask fires.
  assertEquals(store.list(), [{ id: "b" }]);
  assertEquals(store.getSnapshot(), [{ id: "b" }]);

  await Promise.resolve();
  // Listeners must still be notified even though getSnapshot() rebuilt earlier.
  assertEquals(notifications, 1);

  store.map.set("c", { id: "c" });
  store.notify();
  store.map.set("d", { id: "d" });
  store.notify();
  await Promise.resolve();

  assertEquals(notifications, 2);
  assertEquals(store.getSnapshot(), [{ id: "b" }, { id: "c" }, { id: "d" }]);
  unsubscribe();
});

test("commandRegistryStore publishes distinct command references when an approval slot updates", async () => {
  const store = createCommandRegistryStore<{ id: string; title: string }>();
  let notifications = 0;
  const unsubscribe = store.subscribe(() => {
    notifications += 1;
  });

  const first = { id: "approval.choice.1", title: "Allow once" };
  store.map.set(first.id, first);
  store.notify();
  await Promise.resolve();
  assertEquals(notifications, 1);
  assertEquals(store.list().find((entry) => entry.id === first.id) === first, true);

  const expanded = { id: "approval.choice.1", title: "Allow these calls" };
  store.map.set(expanded.id, expanded);
  store.notify();
  // Even before microtask flush, live lookup rejects the stale row reference.
  assertEquals(store.list().find((entry) => entry.id === first.id) === first, false);
  await Promise.resolve();
  assertEquals(notifications, 2);
  assertEquals(store.getSnapshot()[0], expanded);
  unsubscribe();
});
