/** Real browser streams/cancellation; synthetic HTTP, no native/device claim. */
import { ID, OTHER, wire } from "./codeBuffers/fixture.ts";
import {
  destinationWire,
  golden,
  handedOff,
  retainedNavigation,
  targetText,
} from "./codeBuffers/destinationFixture.ts";
import { NAV_ID } from "./codeBuffers/navigationFixture.ts";
import { BufferClientError } from "./codeBuffers/protocol.ts";

function check(value: unknown): asserts value {
  if (!value) throw new Error("destination browser fixture failed");
}
async function calls(
  f: Awaited<ReturnType<typeof retainedNavigation>>,
  count: number,
) {
  for (let n = 0; n < 300 && f.calls.length < count; n++) {
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  check(f.calls.length === count);
}
async function refusal(task: Promise<unknown>, kind: string) {
  try {
    await task;
  } catch (error) {
    check(error instanceof BufferClientError && error.kind === kind);
    return;
  }
  throw new Error("destination unexpectedly succeeded");
}

export async function runDestinationBrowserConformance(): Promise<string[]> {
  const tests: string[] = [];
  const f = await retainedNavigation();
  const button = document.createElement("button");
  document.body.append(button);
  try {
    const tasks: Promise<unknown>[] = [];
    button.onclick = () => tasks.push(f.operation.prepareDestination(f.target));
    button.click();
    button.click();
    await refusal(tasks[1]!, "busy");
    const child = f.operation.destination(f.target)!;
    check(child.view().handingOff && f.registry.retained().includes(child));
    check(f.calls[4]!.url === `/api/code/navigations/${NAV_ID}/destinations`);
    check(
      JSON.stringify(JSON.parse(f.calls[4]!.init.body as string)) ===
        JSON.stringify({ destination: 0, content: f.target.location.content }),
    );
    f.reply(4, golden);
    check(await tasks[0] === child && f.calls.length === 5);
    const open = child.open();
    f.reply(5, wire("open", OTHER));
    await open;
    const release = f.operation.release();
    f.reply(6, { ...golden, state: "released" });
    await release;
    const sourceClose = f.owner.close();
    await calls(f, 8);
    f.reply(7, wire("open"));
    await calls(f, 9);
    f.reply(8, wire("released"));
    await sourceClose;
    check(
      f.registry.retained().length === 1 && f.registry.retained()[0] === child,
    );
    const read = child.readText(f.target.location.content);
    f.reply(9, targetText());
    const text = await read;
    check(text.kind === "complete" && text.content.text === "abc");
    const close = child.close();
    await calls(f, 11);
    f.reply(10, wire("released", OTHER));
    check(
      (await close).kind === "released" && f.registry.retained().length === 0,
    );
    tests.push(
      "destination browser double-click reserves once; explicit child Open and native-text read survive parent/source release",
    );
  } finally {
    f.context.abort();
    button.remove();
  }

  const cancelled = await retainedNavigation();
  try {
    const observer = new AbortController();
    const preparing = cancelled.operation.prepareDestination(
      cancelled.target,
      observer.signal,
    );
    const ended = refusal(preparing, "cancelled");
    observer.abort();
    await ended;
    const child = cancelled.operation.destination(cancelled.target)!;
    check((await child.close()).kind === "retained");
    check(cancelled.registry.cleanup.get().rows[0]?.status === "destination");
    const closing = cancelled.owner.close();
    cancelled.reply(4, golden);
    check((await closing).kind === "retained");
    check(
      child.view().resourceId === OTHER &&
        !cancelled.calls[4]!.init.signal!.aborted,
    );
    await refusal(child.open(), "state");
    check(cancelled.calls.length === 5);
    const cleanup = child.close();
    await calls(cancelled, 6);
    cancelled.reply(5, wire("released", OTHER));
    await cleanup;
    const query = cancelled.operation.observe();
    cancelled.reply(6, golden);
    await query;
    check(
      child.view().phase === "released" &&
        !cancelled.registry.retained().includes(child),
    );
    tests.push(
      "cancelled destination handoff drains a late ID after both views close; historical parent receipt cannot revive the released child",
    );
  } finally {
    cancelled.context.abort();
  }

  const lost = await retainedNavigation();
  try {
    const preparing = lost.operation.prepareDestination(lost.target);
    lost.calls[4]!.result.reject(new Error("discarded reply"));
    await refusal(preparing, "transport");
    const child = lost.operation.destination(lost.target)!;
    check((await child.close()).kind === "retained");
    const unknown = lost.operation.observe();
    lost.reply(5, destinationWire("unknown", true), 202);
    await unknown;
    await refusal(lost.operation.prepareDestination(lost.target), "state");
    const query = lost.operation.observe();
    lost.reply(6, golden);
    await query;
    check(
      lost.operation.destination(lost.target) === child &&
        child.view().resourceId === OTHER,
    );
    check(
      lost.calls.filter(({ url }) => url.endsWith("/destinations")).length ===
        1,
    );
    tests.push(
      "lost destination POST retains its original capacity and recovers by group Query through pending Unknown without replay",
    );
  } finally {
    lost.context.abort();
  }

  const bounded = await retainedNavigation();
  try {
    await refusal(
      bounded.operation.prepareDestination(structuredClone(bounded.target)),
      "state",
    );
    const slots = Array.from(
      { length: 63 },
      (_, n) =>
        bounded.registry.reserve({ sessionId: "other", path: `file${n}` }),
    );
    await refusal(
      bounded.operation.prepareDestination(bounded.target),
      "capacity",
    );
    check(
      bounded.calls.length === 4 &&
        !bounded.operation.destination(bounded.target),
    );
    await slots[0]!.close();
    const preparing = bounded.operation.prepareDestination(bounded.target);
    bounded.reply(4, destinationWire("unknown"));
    const child = await preparing;
    check(
      (await child.close()).kind === "retained" &&
        bounded.registry.retained().length === 64,
    );
    const expiry = bounded.operation.observe();
    bounded.reply(5, destinationWire("expired"));
    await expiry;
    check(
      bounded.registry.retained().length === 63 &&
        child.view().phase === "unopened",
    );
    await refusal(
      bounded.operation.prepareDestination(bounded.target),
      "state",
    );
    tests.push(
      "browser cloned target cannot grant ownership; pending destination counts against capacity until confirmed inert expiry",
    );
  } finally {
    bounded.context.abort();
  }

  const historical = await handedOff();
  try {
    const opening = historical.child.open();
    historical.reply(5, wire("open", OTHER));
    await opening;
    const bad = historical.operation.observe();
    historical.reply(6, {
      ...golden,
      destinations: [{ destination: 0, state: "prepared", resourceId: ID }],
    });
    await refusal(bad, "protocol");
    check(
      historical.child.view().resourceId === OTHER &&
        historical.child.view().observation?.state === "open",
    );
    const query = historical.operation.observe();
    historical.reply(7, golden);
    await query;
    check(
      historical.child.view().observation?.state === "open" &&
        historical.calls.length === 8,
    );
    tests.push(
      "browser refuses substituted destination identity and never resets a live child from historical Prepared evidence",
    );
  } finally {
    historical.context.abort();
  }

  const revoked = await retainedNavigation();
  try {
    const preparing = revoked.operation.prepareDestination(revoked.target);
    const ended = refusal(preparing, "context_lost");
    const child = revoked.operation.destination(revoked.target)!;
    revoked.context.abort();
    await ended;
    check((await child.close()).kind === "retained" && child.view().handingOff);
    check(
      revoked.registry.cleanup.get().rows.every((row) =>
        row.target === undefined && !row.canInspect && !row.canContinue
      ),
    );
    await refusal(revoked.operation.observe(), "context_lost");
    check(revoked.calls.length === 5);
    tests.push(
      "core identity loss retains unresolved destination ownership and redacts all recovery labels without fresh account adoption",
    );
  } finally {
    revoked.context.abort();
  }
  return tests;
}
