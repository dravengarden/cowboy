/** Actual core identity/dataset/session-end functions in a fresh browser.
 * HTTP is synthetic; this does not exercise password login, Review or native LSP.
 */
import { createElement, StrictMode, useEffect, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { replicatedStore } from "@cowboy/state-sync";
import { createSyncShutdown } from "./syncShutdown.ts";
import {
  createProductCodeBuffers,
  productCodeBuffers,
} from "./codeBuffers/product.ts";
import type { OwnedCodeBuffer } from "./codeBuffers/owner.ts";
import { BufferClientError, type BufferState } from "./codeBuffers/protocol.ts";
import { deferred, readWire, wire } from "./codeBuffers/fixture.ts";
import {
  bindProductSyncPrincipal,
  productSyncPrincipal,
} from "./productSyncIdentity.ts";
import {
  createProductSyncDatabase,
  productSyncDatabase,
} from "./productSyncDatabase.ts";
import {
  announceProductSessionEnd,
  PRODUCT_SESSION_END_EVENT,
  ProductSessionEndEvent,
  productSessionSignal,
} from "./productSessionEnd.ts";

function check(value: unknown, detail: string): asserts value {
  if (!value) throw new Error(detail);
}
async function refused(operation: Promise<unknown>, kind?: string) {
  try {
    await operation;
  } catch (error) {
    if (kind) {
      check(
        error instanceof BufferClientError && error.kind === kind,
        "unexpected refusal",
      );
    }
    return;
  }
  throw new Error("expected refusal");
}
async function until(predicate: () => boolean) {
  for (let n = 0; n < 300; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error("context fixture timed out");
}
function descriptor(key = "a") {
  return {
    schema: "dravengarden.cowboy.product-sync-dataset/v1" as const,
    dataset_id: `dataset-${key.repeat(64)}`,
    user_id: "fixture-user",
    database_version: 2 as const,
    outbox_contract: "atomic-delta-v1" as const,
  };
}

export async function runCodeBufferContextBrowserConformance(): Promise<
  string[]
> {
  const tests: string[] = [];
  const originalFetch = globalThis.fetch;
  let discoveries = 0, status = 200, counter = 0;
  const discoveryCount = () => discoveries;
  let pendingRead: ReturnType<typeof deferred<Response>> | undefined;
  const calls: {
    method: string;
    url: string;
    signal: AbortSignal | null | undefined;
  }[] = [];
  const states = new Map<string, BufferState>();
  const fetchBuffer = (url: string, init: RequestInit) => {
    calls.push({ method: init.method!, url, signal: init.signal });
    let id = url.split("/")[4]!;
    if (url === "/api/code/buffers" && init.method === "POST") {
      id = `0123456789abcdef0123456789abcdef-${
        (++counter).toString(16).padStart(16, "0")
      }`;
      states.set(id, "prepared");
    } else if (url.endsWith("/read")) {
      check(states.get(id) === "open", "read wrong owner");
      return pendingRead?.promise ??
        Promise.resolve(Response.json(readWire("language", id)));
    } else if (init.method === "PUT") {
      check(states.get(id) === "prepared", "replayed open");
      states.set(id, "open");
    } else if (init.method === "DELETE") {
      check(states.has(id), "foreign cleanup");
      states.set(id, "released");
    }
    check(states.has(id), "unknown resource");
    return Promise.resolve(Response.json(wire(states.get(id)!, id)));
  };
  const product = createProductCodeBuffers(productSyncDatabase, {
    fetch: fetchBuffer,
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let unmounted = false, mounts = 0;
  const closing: Promise<unknown>[] = [];
  let drain: () => Promise<void> = () => Promise.resolve();
  function Harness() {
    const [view, setView] = useState("waiting");
    useEffect(() => {
      mounts++;
      const observer = new AbortController();
      let owner: OwnedCodeBuffer | undefined;
      void product.ready(observer.signal).then(async (registry) => {
        if (observer.signal.aborted) return;
        owner = registry.reserve({
          sessionId: "fixture-session",
          path: "fixture.rs",
        });
        await owner.prepare();
        if (observer.signal.aborted) return;
        await owner.open();
        if (!observer.signal.aborted) setView("open");
      }).catch(() => {
        if (!observer.signal.aborted) setView("failed");
      });
      return () => {
        observer.abort();
        if (owner) closing.push(owner.close());
      };
    }, []);
    return createElement("output", {}, view);
  }
  globalThis.fetch = (input, init) => {
    check(
      input === "/api/sync/dataset" && init?.credentials === "same-origin" &&
        init.cache === "no-store",
      "unexpected metadata request",
    );
    discoveries++;
    return Promise.resolve(Response.json(descriptor(), { status }));
  };
  try {
    await refused(productCodeBuffers.ready(), "transport");
    check(
      discoveries === 0 && calls.length === 0,
      "logged-out consumer started effects",
    );
    check(
      !bindProductSyncPrincipal("display label"),
      "display label adopted as identity",
    );
    tests.push(
      "logged-out actual core port and invalid display identity cannot discover or open resources",
    );

    check(
      bindProductSyncPrincipal("fixture-user"),
      "core identity binding failed",
    );
    const [registry, second] = await Promise.all([
      product.ready(),
      product.ready(),
      productCodeBuffers.ready(),
    ]);
    check(
      registry === second && await product.ready() === registry,
      "registry recreated",
    );
    check(
      discoveryCount() === 1 &&
        productSyncDatabase.lifecycle.openRequests === 0 &&
        calls.length === 0,
      "readiness opened storage or buffers",
    );
    tests.push(
      "actual principal and shared dataset discovery produce one reusable core buffer context without native or storage effects",
    );

    flushSync(() =>
      root.render(createElement(StrictMode, {}, createElement(Harness)))
    );
    await until(() =>
      container.querySelector("output")?.textContent === "open"
    );
    check(
      mounts === 2 && counter === 1,
      "StrictMode readiness replay allocated a stale owner",
    );
    flushSync(() => root.unmount());
    unmounted = true;
    await Promise.all(closing);
    check(
      registry.retained().length === 0 && !productSyncDatabase.signal.aborted,
      "view disposal ended the product context",
    );
    tests.push(
      "real StrictMode replay cancels only the old ready observer and view teardown retains the shared product context",
    );

    status = 503;
    await refused(productSyncDatabase.connection());
    check(
      !productSyncDatabase.signal.aborted,
      "temporary outage became authority loss",
    );
    status = 200;
    await productSyncDatabase.connection();
    check(
      await product.ready() === registry,
      "same-Service reconnect replaced the registry",
    );
    tests.push(
      "temporary metadata failure and same-Service reconnect do not recreate the original registry",
    );

    let remote = descriptor();
    const data = createProductSyncDatabase(
      () => "fixture-user",
      () => Promise.resolve(remote),
      { context: productSessionSignal() },
    );
    try {
      const independent = createProductCodeBuffers(data, {
        fetch: fetchBuffer,
      });
      const bound = await independent.ready();
      const old = bound.reserve({
        sessionId: "fixture-session",
        path: "old.rs",
      });
      await old.prepare();
      await old.open();
      const count = calls.length;
      remote = descriptor("b");
      await refused(data.connection());
      remote = descriptor();
      await refused(independent.ready(), "context_lost");
      check(
        (await old.close()).kind === "retained" && calls.length === count,
        "Service replacement sent cleanup or revived an owner",
      );
      tests.push(
        "observed Service replacement irreversibly fences the original buffer context without replay or replacement cleanup",
      );
    } finally {
      await data.dispose();
    }

    const active = registry.reserve({
      sessionId: "fixture-session",
      path: "last.rs",
    });
    await active.prepare();
    await active.open();
    const scope = {
      kind: "session",
      session: "fixture-session",
      state: "queue",
    } as const;
    const writer = replicatedStore({
      initial: 0,
      clientId: "fixture",
      mutators: { add: (value: number, amount: number) => value + amount },
      local: productSyncDatabase.outbox<number>(scope),
      send: () => {},
      saveDebounceMs: 60_000,
    });
    const shutdown = createSyncShutdown(productSyncDatabase);
    drain = () => shutdown([writer]);
    await writer.hydrate();
    writer.mutate("add", 7, "retained-after-session-end");
    pendingRead = deferred<Response>();
    const rejected = refused(active.read("language"), "context_lost");
    const count = calls.length;
    const onEnd = (event: Event) => {
      check(
        productSyncDatabase.signal.aborted && active.view().contextLost &&
          calls.at(-1)!.signal!.aborted,
        "session event preceded the synchronous authority fence",
      );
      if (event instanceof ProductSessionEndEvent) event.waitUntil(drain());
    };
    globalThis.addEventListener(PRODUCT_SESSION_END_EVENT, onEnd, {
      once: true,
    });
    check(
      await announceProductSessionEnd() === "drained",
      "local write did not drain",
    );
    await rejected;
    pendingRead.resolve(
      Response.json(readWire("language", active.view().resourceId!)),
    );
    check(
      (await active.close()).kind === "retained" && calls.length === count,
      "session end implied native release",
    );
    check(
      productSyncPrincipal() === "fixture-user" &&
        !bindProductSyncPrincipal("fixture-user") &&
        !bindProductSyncPrincipal("next-user"),
      "ended root adopted a new principal",
    );
    await refused(productCodeBuffers.ready(), "context_lost");
    const late = createProductCodeBuffers(productSyncDatabase, {
      fetch: fetchBuffer,
    });
    await refused(late.ready(), "context_lost");
    check(
      productSessionSignal().aborted && registry.retained().includes(active),
      "ended lifetime or unresolved evidence was discarded",
    );
    const reader = createProductSyncDatabase(
      () => "fixture-user",
      () => Promise.resolve(descriptor()),
    );
    try {
      const saved = await reader.outbox<number>(scope).load();
      check(
        saved?.pending.length === 1 &&
          saved.pending[0]!.id === "retained-after-session-end",
        "native IDB lost the final outbox write",
      );
    } finally {
      await reader.dispose();
    }
    tests.push(
      "actual session-end fences remote reads and late consumers while the prior local outbox drains durably into native IDB; no native cleanup is inferred",
    );
    return tests;
  } finally {
    if (!unmounted) flushSync(() => root.unmount());
    await drain();
    await productSyncDatabase.dispose();
    await Promise.all(closing);
    globalThis.fetch = originalFetch;
    container.remove();
  }
}
