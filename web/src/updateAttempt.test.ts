import { assert, assertEquals } from "jsr:@std/assert";
import {
  advanceUpdateAttempt,
  clearUpdateAttempt,
  markUpdateSwapping,
  UPDATE_ATTEMPT_KEY,
  UPDATE_ATTEMPT_TTL_MS,
  rolledBackNavigationUrl,
  updateSwapInFlight,
} from "./updateAttempt.ts";

function storage(seed: Record<string, string> = {}) {
  const map = new Map(Object.entries(seed));
  return {
    map,
    getItem: (key: string): string | null => map.get(key) ?? null,
    setItem: (key: string, value: string): void => void map.set(key, value),
    removeItem: (key: string): void => void map.delete(key),
  };
}

Deno.test("a swap signs itself in, then the build signs off", () => {
  const store = storage();
  markUpdateSwapping(store, "cowboy-v1753", 1000);
  assertEquals(updateSwapInFlight(store), true);

  // The load the swap was made for: still swapping, so it takes the pen.
  const first = advanceUpdateAttempt(store.getItem(UPDATE_ATTEMPT_KEY), 1200);
  assertEquals(first.failed, undefined);
  assertEquals(first.next?.phase, "booting");
  assertEquals(first.next?.to, "cowboy-v1753");

  clearUpdateAttempt(store);
  assertEquals(updateSwapInFlight(store), false);
  assertEquals(advanceUpdateAttempt(store.getItem(UPDATE_ATTEMPT_KEY), 1300).failed, undefined);
});

Deno.test("a document that ran and never came up is the failure signature", () => {
  // Exactly one shape means a build did not start: a second load finding
  // `booting`. The first load wrote it and was supposed to erase it.
  const store = storage();
  markUpdateSwapping(store, "cowboy-v1753", 1000);
  const first = advanceUpdateAttempt(store.getItem(UPDATE_ATTEMPT_KEY), 1200);
  store.setItem(UPDATE_ATTEMPT_KEY, JSON.stringify(first.next));

  const second = advanceUpdateAttempt(store.getItem(UPDATE_ATTEMPT_KEY), 1400);
  assertEquals(second.next, undefined);
  assertEquals(second.failed?.to, "cowboy-v1753");
  assertEquals(second.failed?.phase, "booting");
});

Deno.test("a stale or unreadable marker accuses nobody", () => {
  const store = storage();
  markUpdateSwapping(store, "cowboy-v1753", 1000);
  const booting = advanceUpdateAttempt(store.getItem(UPDATE_ATTEMPT_KEY), 1000).next;
  store.setItem(UPDATE_ATTEMPT_KEY, JSON.stringify(booting));

  // A device closed mid-update and reopened a day later did not just crash.
  const late = advanceUpdateAttempt(
    store.getItem(UPDATE_ATTEMPT_KEY),
    1000 + UPDATE_ATTEMPT_TTL_MS + 1,
  );
  assertEquals(late.failed, undefined);
  assertEquals(late.next, undefined);

  // A clock that moved backwards is not evidence either.
  assertEquals(advanceUpdateAttempt(store.getItem(UPDATE_ATTEMPT_KEY), 10).failed, undefined);

  for (const raw of [null, "", "{", "{}", '{"phase":"nonsense","at":1000}', '"string"']) {
    const transition = advanceUpdateAttempt(raw, 1200);
    assertEquals(transition.failed, undefined);
    assertEquals(transition.next, undefined);
  }
});

Deno.test("denied storage loses the guard, never the update", () => {
  const denied = {
    getItem: (): string | null => {
      throw new Error("denied");
    },
    setItem: (): void => {
      throw new Error("denied");
    },
    removeItem: (): void => {
      throw new Error("denied");
    },
  };
  markUpdateSwapping(denied, "cowboy-v1753", 1000);
  clearUpdateAttempt(denied);
  assertEquals(updateSwapInFlight(denied), false);
  markUpdateSwapping(undefined, "cowboy-v1753", 1000);
});

Deno.test("the pre-module ledger in index.html is the same ledger", async () => {
  // The document has to decide this before any module exists, so the rules are
  // written twice. These are the exact strings that make the two copies one.
  const html = await Deno.readTextFile(new URL("../index.html", import.meta.url));
  const ledger = html.slice(
    html.indexOf("Did the build this document belongs to actually start?"),
    html.indexOf("__cowboyUpdateBootFailed") + 64,
  );
  assert(ledger.length > 0, "the inline ledger is gone from index.html");
  assert(ledger.includes(`const KEY = "${UPDATE_ATTEMPT_KEY}";`));
  assert(ledger.includes(`const TTL_MS = ${String(UPDATE_ATTEMPT_TTL_MS / 60000)} * 60000;`));
  assert(ledger.includes(`attempt.phase === "swapping"`));
  assert(ledger.includes(`phase: "booting"`));
  assert(ledger.includes(`attempt.phase === "booting"`));
});

Deno.test("the boot guard rolls back before it renders the tree that failed", async () => {
  const main = await Deno.readTextFile(new URL("./main.tsx", import.meta.url));
  const guard = main.indexOf("__cowboyUpdateBootFailed");
  const render = main.indexOf("createRoot(el).render");
  assert(guard > 0 && render > guard, "the rollback guard must precede the render");

  // A crash inside a swap goes backward, not forward: forward is the build
  // that just crashed.
  const boundary = await Deno.readTextFile(new URL("./AppErrorBoundary.tsx", import.meta.url));
  const swapCheck = boundary.indexOf("updateSwapInFlight(globalThis.localStorage)");
  const forward = boundary.indexOf("if (isModuleLoadError(error)) void this.recover(false);");
  assert(swapCheck > 0 && forward > swapCheck, "the swap check must come first");
});

Deno.test("a rollback navigates away from the network, not toward it", async () => {
  const target = rolledBackNavigationUrl("https://cowboy.test/?session=one#drafts", 1234);
  // A distinct URL, because WKWebView can replay the very document that failed.
  assertEquals(target, "https://cowboy.test/?session=one&cowboy-rolled-back=1234#drafts");

  // And never a param the worker treats as network-first: the network holds
  // exactly the build being run away from.
  const sw = await Deno.readTextFile(new URL("../public/sw.js", import.meta.url));
  const networkParams = /const SHELL_NETWORK_PARAMS = \[([^\]]*)\]/.exec(sw)?.[1] ?? "";
  assert(networkParams.length > 0);
  assert(!networkParams.includes("cowboy-rolled-back"));
  for (const source of ["./main.tsx", "./AppErrorBoundary.tsx"]) {
    const text = await Deno.readTextFile(new URL(source, import.meta.url));
    if (!text.includes("rolledBackNavigationUrl")) continue;
    assert(
      !/rollback[\s\S]{0,400}globalThis\.location\.reload\(\)/.test(text),
      `${source} must navigate, not reload, after a rollback`,
    );
  }
});
