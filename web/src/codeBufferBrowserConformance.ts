/** Real browser/React development StrictMode, deferred HTTP fixtures only.
 * This is NOT Review integration or native-coordinate/physical-device evidence.
 */
import { createElement, StrictMode, useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import golden from "../../contracts/code-buffer-client.fixture.json" with {
  type: "json",
};
import { fixture, ID, opened, OTHER, wire } from "./codeBuffers/fixture.ts";
import type { CloseResult, OwnedCodeBuffer } from "./codeBuffers/owner.ts";
import { BufferClientError } from "./codeBuffers/protocol.ts";
import { captureContent, type CapturedContent } from "./codeBuffers/content.ts";
import contentGolden from "../../plugins/zed/adapter/fixtures/content.json" with {
  type: "json",
};
import { runTextBrowserConformance } from "./codeBufferTextBrowserConformance.ts";
import { runNavigationBrowserConformance } from "./codeBufferNavigationBrowserConformance.ts";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(predicate: () => boolean) {
  for (let attempt = 0; attempt < 300; attempt++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error("browser fixture condition timed out");
}
async function requests(f: ReturnType<typeof fixture>, count: number) {
  // Native browser Response streams can deliver across event-loop tasks;
  // counting only microtasks is a Deno mock assumption, not a timing contract.
  await until(() => f.calls.length >= count);
  check(
    f.calls.length === count,
    `unexpected request count ${f.calls.length}/${count}`,
  );
}
async function refusal(task: Promise<unknown>, kind: string) {
  try {
    await task;
  } catch (error) {
    check(
      error instanceof BufferClientError && error.kind === kind,
      "wrong refusal",
    );
    return;
  }
  throw new Error("expected refusal");
}

async function strictMode(tests: string[]) {
  const f = fixture();
  await f.owner.close(); // This scenario allocates in committed effects only.
  const members: {
    owner: OwnedCodeBuffer;
    closed: Promise<CloseResult> | undefined;
  }[] = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let unmounted = false;
  function Harness({ label }: { label: string }) {
    const [view, setView] = useState(`${label}:preparing`);
    const action = useRef<(() => void) | undefined>(undefined);
    useEffect(() => {
      const owner = f.registry.reserve({
        sessionId: "fixture",
        path: `${label}.rs`,
      });
      const member: typeof members[number] = {
        owner,
        closed: undefined,
      };
      members.push(member);
      let live = true;
      const failed = (error: unknown) => {
        if (
          live && !(error instanceof BufferClientError && error.kind === "busy")
        ) {
          setView(`${label}:failed`);
        }
      };
      void owner.prepare().then(async () => {
        if (!live) return;
        const evidence = await owner.open();
        if (live) setView(`${label}:${evidence.state}`);
      }).catch(failed);
      action.current = () => {
        void owner.read("language").then((observation) => {
          if (live) {
            setView(
              `${label}:diagnostics:${observation.result.diagnostics.length}`,
            );
          }
        }).catch(failed);
      };
      return () => {
        live = false;
        action.current = undefined;
        member.closed = owner.close();
      };
    }, [label]);
    return createElement(
      "section",
      {},
      createElement("output", {}, view),
      createElement("button", { onClick: () => action.current?.() }, "Read"),
    );
  }
  const render = (label: string) =>
    flushSync(() =>
      root.render(
        createElement(
          StrictMode,
          {},
          createElement(Harness, { key: label, label }),
        ),
      )
    );
  const calls = (count: number) => requests(f, count);
  const output = () => container.querySelector("output")?.textContent;
  const click = () => {
    const button = container.querySelector("button");
    check(button instanceof HTMLButtonElement, "missing read button");
    button.click();
  };
  const language = { ...golden.language, resourceId: OTHER };
  try {
    render("first");
    await calls(2);
    check(
      members.length === 2,
      "StrictMode did not replay the committed effect",
    );
    f.reply(0, wire("prepared"));
    await calls(3);
    check(
      f.calls[2]!.init.method === "DELETE" && f.calls[2]!.url.endsWith(ID),
      "old preparation reopened",
    );
    f.reply(2, wire("released"));
    check(
      (await members[0]!.closed)?.kind === "released",
      "old owner not drained",
    );
    f.reply(1, wire("prepared", OTHER));
    await calls(4);
    check(
      f.calls[3]!.init.method === "PUT" && f.calls[3]!.url.endsWith(OTHER),
      "new owner rebound",
    );
    f.reply(3, wire("open", OTHER));
    await until(() => output() === "first:open");
    tests.push(
      "StrictMode replay cleans up a late original reservation without opening it or rebinding the live owner",
    );

    click();
    click();
    await calls(5);
    f.reply(4, language);
    await until(() => output() === "first:diagnostics:1");
    tests.push(
      "same-stack browser clicks admit one read and decode the nonempty Rust wire fixture",
    );

    click();
    await calls(6);
    render("next");
    await calls(8);
    check(
      !f.calls[5]!.init.signal!.aborted,
      "view unmount aborted the owned read",
    );
    f.reply(5, language);
    await calls(9);
    check(
      f.calls[8]!.init.method === "GET" && f.calls[8]!.url.endsWith(OTHER),
      "old read did not reconcile its original owner",
    );
    f.reply(8, wire("open", OTHER));
    await calls(10);
    f.reply(9, wire("released", OTHER));
    check(
      (await members[1]!.closed)?.kind === "released",
      "retired view not cleaned up",
    );
    check(
      output() === "next:preparing",
      "late diagnostics poisoned replacement view",
    );
    const third = "0123456789abcdef0123456789abcdef-0000000000000003";
    const fourth = "0123456789abcdef0123456789abcdef-0000000000000004";
    f.reply(6, wire("prepared", third));
    await calls(11);
    f.reply(10, wire("released", third));
    await members[2]!.closed;
    f.reply(7, wire("prepared", fourth));
    await calls(12);
    f.reply(11, wire("open", fourth));
    await until(() => output() === "next:open");
    flushSync(() => root.unmount());
    unmounted = true;
    await calls(13);
    f.reply(12, wire("released", fourth));
    check(
      (await members[3]!.closed)?.kind === "released",
      "final unmount not cleaned up",
    );
    check(
      f.registry.retained().length === 0,
      "acknowledged owners not retired",
    );
    tests.push(
      "unmount during a read drains the borrow and suppresses stale UI updates across a second StrictMode mount",
    );
  } finally {
    if (!unmounted) flushSync(() => root.unmount());
    f.context.abort();
    await Promise.all(members.map((member) => member.closed));
    container.remove();
  }
}

export async function runCodeBufferBrowserConformance(): Promise<string[]> {
  const tests: string[] = [];
  await strictMode(tests);
  await contentLifetime(tests);
  tests.push(...await runTextBrowserConformance());
  tests.push(...await runNavigationBrowserConformance());
  const pending = await opened();
  try {
    const close = pending.owner.close();
    await requests(pending, 3);
    pending.reply(2, wire("open", ID, true), 202);
    check((await close).kind === "retained", "202 was treated as released");
    await new Promise<void>((resolve) => setTimeout(resolve, 20));
    check(pending.calls.length === 3, "cleanup silently polled/retried");
    const retry = pending.owner.close();
    await requests(pending, 4);
    check(
      pending.calls[3]!.init.method === "GET",
      "cleanup did not observe first",
    );
    pending.reply(3, wire("open"));
    await requests(pending, 5);
    pending.reply(4, wire("released"));
    check(
      (await retry).kind === "released",
      "explicit cleanup did not complete",
    );
    tests.push(
      "202 cleanup remains retained and requires an explicit observed cleanup pass",
    );
  } finally {
    pending.context.abort();
  }
  const lost = fixture();
  try {
    const prepare = lost.owner.prepare();
    lost.reply(0, wire("prepared"));
    await prepare;
    const open = lost.owner.open();
    lost.calls[1]!.result.reject(new TypeError("fixture connection lost"));
    await refusal(open, "transport");
    const observe = lost.owner.observe();
    lost.reply(2, wire("prepared"));
    await observe;
    await refusal(lost.owner.open(), "state");
    const close = lost.owner.close();
    await requests(lost, 4);
    lost.reply(3, wire("released"));
    check(
      (await close).kind === "released" && lost.calls.length === 4,
      "lost open was replayed",
    );
    tests.push(
      "lost open is queried by original id and never replayed even after a prepared observation",
    );
  } finally {
    lost.context.abort();
  }
  const authority = await opened();
  try {
    const read = authority.owner.read("language");
    const denied = refusal(read, "context_lost");
    authority.context.abort();
    authority.reply(2, golden.language);
    await denied;
    check(
      authority.calls[2]!.init.signal!.aborted,
      "authority abort did not fence transport",
    );
    check(
      (await authority.owner.close()).kind === "retained",
      "authority loss claimed release",
    );
    await refusal(authority.owner.observe(), "context_lost");
    check(
      authority.calls.length === 3 &&
        authority.registry.retained().length === 1,
      "old owner adopted replacement authority",
    );
    tests.push(
      "authority lifetime loss aborts transport, fences cleanup and retains unresolved original ownership",
    );
  } finally {
    authority.context.abort();
  }
  return tests;
}

async function contentLifetime(tests: string[]) {
  const f = await opened();
  const first = await captureContent(contentGolden.text);
  const second = await captureContent("different text\n");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const outcomes: string[] = [];
  let action: (() => void) | undefined;
  function Harness({ snapshot }: { snapshot: CapturedContent }) {
    const [result, setResult] = useState("idle");
    useEffect(() => {
      const view = new AbortController();
      action = () => {
        void f.owner.readContent(snapshot, {
          kind: "hover",
          position: { row: 0, column: 1 },
        }, view.signal)
          .then((value) => {
            if (!view.signal.aborted) {
              outcomes.push(value.result.result.kind);
              setResult(value.result.result.kind);
            }
          }).catch((error: unknown) => {
            check(
              error instanceof BufferClientError && error.kind === "cancelled",
              "unexpected content error",
            );
          });
      };
      return () => {
        view.abort();
        action = undefined;
      };
    }, [snapshot]);
    return createElement("button", { onClick: () => action?.() }, result);
  }
  const render = (snapshot: CapturedContent) =>
    flushSync(() =>
      root.render(
        createElement(StrictMode, null, createElement(Harness, { snapshot })),
      )
    );
  try {
    render(first);
    container.querySelector("button")!.click();
    await requests(f, 3);
    render(second);
    f.reply(2, contentGolden.response);
    await until(() => !f.owner.view().busy);
    check(
      outcomes.length === 0 && container.textContent === "idle",
      "old text result painted into replacement view",
    );
    // Discarding a stale read requires observing the original owner again.
    const observation = f.owner.observe();
    f.reply(3, wire("open"));
    await observation;
    container.querySelector("button")!.click();
    await requests(f, 5);
    const sent = JSON.parse(String(f.calls[4]!.init.body));
    check(
      sent.content.sha256 !== contentGolden.request.content.sha256,
      "replacement reused old text digest",
    );
    f.reply(4, {
      ...contentGolden.response,
      result: {
        kind: "content",
        content: sent.content,
        result: { kind: "mismatch" },
      },
    });
    await until(() => container.textContent === "mismatch");
    check(
      outcomes.join() === "mismatch" && f.calls.length === 5,
      "mismatch auto-reloaded or reopened native owner",
    );
    tests.push(
      "real WebCrypto hashes displayed Unicode text and StrictMode content replacement discards an in-flight hover",
    );
    tests.push(
      "content mismatch is distinct from empty hover and never triggers reload or replacement ownership",
    );
    flushSync(() => root.unmount());
    const closed = f.owner.close();
    await requests(f, 6);
    f.reply(5, wire("released"));
    check(
      (await closed).kind === "released",
      "original content owner did not drain",
    );
  } finally {
    f.context.abort();
    root.unmount();
    container.remove();
  }
}
