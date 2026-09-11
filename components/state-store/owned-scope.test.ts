import {
  assertEquals,
  assertRejects,
  assertStrictEquals,
  assertThrows,
} from "jsr:@std/assert";
import { createOwnedResourceScope, ScopeClosedError } from "./owned-scope.ts";

Deno.test("owned scope seals synchronously and releases consumer before provider", async () => {
  const scope = createOwnedResourceScope();
  const order: string[] = [];
  scope.defer(() => {
    order.push("provider");
  });
  scope.defer(async () => {
    await Promise.resolve();
    order.push("consumer");
  });
  const notify = scope.guard(() => {
    order.push("observation");
  });
  notify();
  const disposed = scope.dispose();
  assertStrictEquals(scope.dispose(), disposed);
  assertEquals(scope.signal.aborted, true);
  assertEquals(scope.snapshot().phase, "draining");
  assertThrows(() => scope.defer(() => {}), ScopeClosedError);
  assertThrows(() => scope.run(() => Promise.resolve()), ScopeClosedError);
  notify();
  await disposed;
  assertEquals(order, ["observation", "consumer", "provider"]);
  assertEquals(scope.snapshot(), {
    phase: "disposed",
    tasks: 0,
    resources: 0,
    failures: 0,
  });
});

Deno.test("owned scope drains admitted tasks before releasing their dependencies", async () => {
  const scope = createOwnedResourceScope();
  const task = Promise.withResolvers<void>();
  let released = false;
  scope.defer(() => {
    released = true;
  });
  const running = scope.run(() => task.promise);
  const done = scope.dispose();
  await Promise.resolve();
  assertEquals(scope.snapshot().tasks, 1);
  assertEquals(released, false);
  task.reject(new Error("task failed, not a resource cleanup failure"));
  await assertRejects(() => running);
  await done;
  assertEquals(released, true);
  assertEquals(scope.snapshot().failures, 0);
});

Deno.test("early release is idempotent and failures stay visible without preventing other cleanup", async () => {
  const scope = createOwnedResourceScope();
  let attempts = 0;
  let otherReleased = false;
  const release = scope.defer(() => {
    attempts++;
    throw new Error("private detail");
  });
  scope.defer(() => {
    otherReleased = true;
  });
  const failed = release();
  assertStrictEquals(release(), failed);
  await assertRejects(() => failed);
  const done = scope.dispose();
  const error = await assertRejects(() => done, AggregateError);
  assertEquals(error.errors.length, 1);
  assertEquals(attempts, 1);
  assertEquals(otherReleased, true);
  assertEquals(scope.snapshot(), {
    phase: "needs_reconcile",
    tasks: 0,
    resources: 1,
    failures: 1,
  });
  assertStrictEquals(scope.dispose(), done);
});

Deno.test("release completion and abort reentrancy cannot revive or double-dispose ownership", async () => {
  const scope = createOwnedResourceScope();
  let calls = 0;
  let nested: Promise<void> | undefined;
  scope.signal.addEventListener("abort", () => {
    nested = scope.dispose();
  });
  const release = scope.defer(() => {
    calls++;
    void release();
  });
  await release();
  const done = scope.dispose();
  assertStrictEquals(nested, done);
  await done;
  assertEquals(calls, 1);
  assertEquals(scope.snapshot().resources, 0);
});

Deno.test("two instances have separate resources, task counts and callback generations", async () => {
  const first = createOwnedResourceScope();
  const second = createOwnedResourceScope();
  let count = 0;
  const stale = first.guard(() => {
    count += 100;
  });
  const live = second.guard((amount: number) => {
    count += amount;
  });
  first.defer(() => {});
  second.defer(() => {});
  await first.dispose();
  stale();
  live(3);
  assertEquals(count, 3);
  assertEquals(second.snapshot(), {
    phase: "active",
    tasks: 0,
    resources: 1,
    failures: 0,
  });
  await second.dispose();
});
