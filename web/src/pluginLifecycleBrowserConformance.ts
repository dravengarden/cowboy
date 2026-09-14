/** Real core history UI; every fetch is an owned, deferred synthetic GET. */
import { createElement, StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { PluginLifecycleHistory } from "./PluginLifecycleHistory.tsx";
import { lifecycleFixture } from "./pluginLifecycle.fixture.ts";
import { PRODUCT_SESSION_END_EVENT } from "./productSessionEnd.ts";
import { deferredFixture } from "./providerManagement.fixture.ts";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function settle() {
  await new Promise<void>((resolve) => setTimeout(resolve, 25));
}

export async function runPluginLifecycleBrowserConformance(): Promise<
  string[]
> {
  const previous = globalThis.fetch;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const calls: {
    target: string;
    init: RequestInit;
    reply: ReturnType<typeof deferredFixture<Response>>;
  }[] = [];
  const tests: string[] = [];
  const requestCount = () => calls.length;
  const callAt = (index: number) => {
    const call = calls[index];
    check(call, "missing expected history read");
    return call;
  };
  globalThis.fetch = ((target, init = {}) => {
    check(
      typeof target === "string" &&
        /^\/api\/machines\/(hawk|other)\/plugins\/victoria\/lifecycle-history$/
          .test(target),
      "unexpected history route",
    );
    check(
      (init.method ?? "GET") === "GET" && init.body === undefined &&
        init.cache === "no-store",
      "history attempted an effect",
    );
    const reply = deferredFixture<Response>();
    calls.push({ target, init, reply });
    return reply.promise;
  }) as typeof fetch;
  const render = (machine = "hawk") =>
    flushSync(() =>
      root.render(
        createElement(
          StrictMode,
          null,
          createElement(PluginLifecycleHistory, {
            machine,
            plugin: "victoria",
          }),
        ),
      )
    );
  const click = (text: string) => {
    const button = [...container.querySelectorAll("button")].find((button) =>
      button.textContent === text
    );
    check(button, `missing history button: ${text}`);
    flushSync(() => button.click());
  };
  const answer = (index: number, machine = "hawk") =>
    callAt(index).reply.resolve(Response.json(lifecycleFixture(machine)));
  try {
    render();
    await settle();
    check(requestCount() === 0, "collapsed view performed a read");
    click("Plugin operation history");
    await settle();
    check(requestCount() === 1, "open must perform exactly one GET");
    answer(0);
    await settle();
    check(
      container.textContent?.includes("Install ·") &&
        container.textContent.includes("Uninstall ·") &&
        container.textContent.includes("resolution-separate-fixture"),
      "domain collision lost an entry or resolution",
    );
    tests.push(
      "StrictMode / closed view is inert / one GET / domain-disjoint rows and independent resolution",
    );

    click("Refresh evidence");
    await settle();
    check(requestCount() === 2, "refresh repeated or replayed an operation");
    answer(1);
    await settle();
    check(
      container.textContent?.includes("independent durable observations") &&
        container.textContent.includes("native turns") === false,
      "evidence semantics missing",
    );
    tests.push(
      "refresh reads the same durable evidence without POST, replay or fabricated completion",
    );

    click("Refresh evidence");
    await settle();
    render("other");
    check(
      !container.textContent?.includes("operation-shared-fixture"),
      "old target painted before effect cleanup",
    );
    await settle();
    check(
      requestCount() === 4 && callAt(2).init.signal?.aborted,
      "target replacement retained the old request",
    );
    answer(2);
    await settle();
    check(
      !container.textContent?.includes("operation-shared-fixture"),
      "late old target response painted",
    );
    answer(3, "other");
    await settle();
    check(
      container.textContent?.includes("operation-shared-fixture"),
      "new target did not load",
    );
    tests.push(
      "target replacement seals old request and prevents old/late evidence paint",
    );

    click("Refresh evidence");
    await settle();
    callAt(4).reply.resolve(new Response("private error", { status: 503 }));
    await settle();
    check(
      container.textContent?.includes("does not mean no operation exists") &&
        !container.textContent.includes("No saved attempts") &&
        !container.textContent.includes("private error"),
      "failed read became absence or leaked its response",
    );
    check(requestCount() === 5, "error caused automatic retry");
    tests.push(
      "failed reads remain unavailable, never empty inventory or automatic retry",
    );

    click("Refresh evidence");
    await settle();
    globalThis.dispatchEvent(new Event(PRODUCT_SESSION_END_EVENT));
    check(
      callAt(5).init.signal?.aborted,
      "product session end did not synchronously seal read",
    );
    answer(5, "other");
    await settle();
    check(
      !container.textContent?.includes("operation-shared-fixture"),
      "late logout response painted",
    );
    tests.push(
      "product session end aborts the owned read before a late completion",
    );

    flushSync(() => root.render(null));
    render();
    await settle();
    click("Plugin operation history");
    await settle();
    check(
      requestCount() === 7,
      "remount replayed an action or retained authority",
    );
    flushSync(() => root.render(null));
    check(callAt(6).init.signal?.aborted, "unmount retained request");
    answer(6);
    await settle();
    check(!container.textContent, "unmounted view repainted");
    tests.push(
      "fresh mount has no stored confirmation / unmount drains late callbacks without replay",
    );
    return tests;
  } finally {
    flushSync(() => root.unmount());
    for (const call of calls) {
      call.reply.resolve(Response.json(lifecycleFixture()));
    }
    await settle();
    globalThis.fetch = previous;
    container.remove();
  }
}
