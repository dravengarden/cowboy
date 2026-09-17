/** Real browser cancellation/streams/WebCrypto, synthetic core HTTP only.
 * No intended Review destination consumer, native budget or device acceptance.
 */
import { opened, wire } from "./codeBuffers/fixture.ts";
import {
  content,
  golden,
  NAV_ID,
  navigationWire,
  preparedNavigation,
} from "./codeBuffers/navigationFixture.ts";
import { BufferClientError } from "./codeBuffers/protocol.ts";

function check(value: unknown): asserts value {
  if (!value) throw new Error("navigation browser fixture failed");
}
async function calls(f: Awaited<ReturnType<typeof opened>>, count: number) {
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
  throw new Error("navigation unexpectedly succeeded");
}

export async function runNavigationBrowserConformance(): Promise<string[]> {
  const tests: string[] = [];
  const f = await preparedNavigation();
  const button = document.createElement("button");
  document.body.append(button);
  try {
    const tasks: Promise<unknown>[] = [];
    button.onclick = () => tasks.push(f.operation.execute());
    button.click();
    button.click();
    await refusal(tasks[1]!, "busy");
    check(
      f.calls.length === 4 &&
        f.calls[3]!.url === `/api/code/navigations/${NAV_ID}`,
    );
    f.reply(3, navigationWire("retained"));
    await tasks[0];
    check(
      f.operation.view().observation.locations[0]?.content.sha256 ===
        golden.content.sha256,
    );
    await refusal(f.operation.execute(), "state");
    await refusal(f.owner.readText(golden.content), "state");
    check((await f.owner.close()).kind === "retained" && f.calls.length === 4);
    tests.push(
      "navigation same-stack browser clicks acquire once and retain the original source fence",
    );
  } finally {
    f.context.abort();
    button.remove();
  }

  const g = await opened();
  try {
    const observer = new AbortController();
    const prepare = g.owner.prepareNavigation(
      await content(),
      golden.position,
      "definition",
      observer.signal,
    );
    const cancelled = refusal(prepare, "cancelled");
    observer.abort();
    await cancelled;
    const closing = g.owner.close();
    g.reply(2, navigationWire());
    check((await closing).kind === "retained");
    const operation = g.owner.navigation()!;
    check(
      !g.calls[2]!.init.signal!.aborted && !operation.view().canExecute &&
        operation.view().canRelease,
    );
    check(g.calls.length === 3);
    tests.push(
      "cancelled navigation preparation stays attached to its closed original view without Execute",
    );
  } finally {
    g.context.abort();
  }

  const h = await preparedNavigation();
  try {
    const observer = new AbortController();
    const execute = h.operation.execute(observer.signal);
    const cancelled = refusal(execute, "cancelled");
    observer.abort();
    await cancelled;
    const closing = h.owner.close();
    h.reply(3, navigationWire("retained"));
    check((await closing).kind === "retained");
    check(!h.calls[3]!.init.signal!.aborted && h.operation.view().canRelease);
    check(
      h.calls.length === 4 &&
        h.registry.cleanup.get().rows[0]?.status === "navigation",
    );
    tests.push(
      "unmount during navigation Execute drains the admitted acquisition without releasing its source",
    );
  } finally {
    h.context.abort();
  }

  const lost = await preparedNavigation();
  try {
    const execute = lost.operation.execute();
    lost.calls[3]!.result.reject(new Error("discarded reply"));
    await refusal(execute, "transport");
    await refusal(lost.operation.execute(), "state");
    await refusal(lost.operation.release(), "state");
    check((await lost.owner.close()).kind === "retained");
    const query = lost.source.inspect(lost.row.handle);
    check(
      lost.calls[4]!.url === `/api/code/navigations/${NAV_ID}` &&
        lost.calls[4]!.init.method === "GET",
    );
    lost.reply(4, navigationWire("retained"));
    await query;
    const release = lost.source.release(lost.row.handle);
    lost.reply(5, navigationWire("released"));
    await release;
    const closing = lost.owner.close();
    await calls(lost, 7);
    check(lost.calls[6]!.init.method === "GET");
    lost.reply(6, wire("open"));
    await calls(lost, 8);
    lost.reply(7, wire("released"));
    check(
      (await closing).kind === "released" &&
        lost.registry.retained().length === 0,
    );
    tests.push(
      "lost navigation Execute queries the original group before separate group and source release",
    );
  } finally {
    lost.context.abort();
  }

  const releasing = await preparedNavigation();
  try {
    const execute = releasing.operation.execute();
    releasing.reply(3, navigationWire("retained"));
    await execute;
    const release = releasing.operation.release();
    releasing.calls[4]!.result.reject(new Error("discarded release"));
    await refusal(release, "transport");
    const query = releasing.operation.observe();
    releasing.reply(5, navigationWire("retained"));
    await query;
    await refusal(releasing.operation.release(), "state");
    const bad = releasing.operation.observe();
    releasing.reply(6, { ...navigationWire("released"), locations: [] });
    await refusal(bad, "protocol");
    const terminal = releasing.operation.observe();
    releasing.reply(7, navigationWire("released"));
    await terminal;
    check(
      releasing.calls.filter(({ init }) => init.method === "DELETE").length ===
        1,
    );
    tests.push(
      "lost navigation Release remains query-only and cannot erase or replace acquired targets",
    );
  } finally {
    releasing.context.abort();
  }

  const pending = await preparedNavigation();
  try {
    const release = pending.operation.release();
    pending.reply(3, navigationWire("prepared", true), 202);
    await release;
    check(
      !pending.operation.view().canRelease &&
        !pending.operation.view().releaseAttempted,
    );
    check(
      (await pending.owner.close()).kind === "retained" &&
        pending.calls.length === 4,
    );
    const query = pending.operation.observe();
    pending.reply(4, navigationWire());
    await query;
    const second = pending.operation.release();
    pending.reply(5, { ...navigationWire("released"), locations: [] });
    await second;
    check(pending.owner.navigation() === undefined);
    tests.push(
      "navigation 202 no-admission requires fresh explicit observation before a separate Release",
    );
  } finally {
    pending.context.abort();
  }

  const revoked = await preparedNavigation();
  try {
    const query = revoked.operation.observe();
    const ended = refusal(query, "context_lost");
    revoked.context.abort();
    await ended;
    const row = revoked.source.get().rows[0]!;
    check(row.target === undefined && !row.canInspect && !row.canRelease);
    await refusal(revoked.operation.execute(), "context_lost");
    check(revoked.calls.length === 4);
    tests.push(
      "navigation recovery redacts source labels and rejects all actions after core identity ends",
    );
  } finally {
    revoked.context.abort();
  }
  return tests;
}
