/** Refresh and cancellation against the actual tree, including its memory cache. */
import { StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";
import { ReviewFileTree } from "./mobile/review/ReviewFileTree.tsx";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 50));
async function until(predicate: () => boolean, label: string) {
  for (let n = 0; n < 300; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`Tree fixture timed out: ${label}`);
}
const noop = () => undefined;
const response = (value: unknown) => Response.json(value);
const page = (path: string, version: number) => ({
  apiVersion: 1,
  revision: `revision-${version}`,
  path,
  truncated: false,
  entries: path === "src/nested"
    ? [{
      name: `file-${version}.ts`,
      path: `src/nested/file-${version}.ts`,
      kind: "file",
      ignored: false,
    }]
    : [{
      name: path ? "nested" : "src",
      path: path ? "src/nested" : "src",
      kind: "directory",
      ignored: false,
    }],
});
const search = (query: string, version: number) => ({
  apiVersion: 1,
  files: [`${query}-${version}.ts`],
});

export async function runReviewTreeBrowserConformance(): Promise<string[]> {
  const tests: string[] = [];
  const originalFetch = globalThis.fetch;
  let version = 1;
  let hold: (url: URL) => boolean = () => false;
  const pending: Array<{
    url: URL;
    signal: AbortSignal | null | undefined;
    resolve: (response: Response) => void;
  }> = [];
  globalThis.fetch = (input, init) => {
    const url = new URL(String(input), location.href);
    if (hold(url)) {
      // Exercise already completed reads too: a late reply may ignore abort.
      return new Promise((resolve) =>
        pending.push({ url, signal: init?.signal, resolve })
      );
    }
    const body = url.pathname.endsWith("/search")
      ? search(url.searchParams.get("q") ?? "", version)
      : page(url.searchParams.get("path") ?? "", version);
    return Promise.resolve(response(body));
  };
  const container = document.createElement("div");
  container.style.cssText = "width:420px;height:600px";
  document.body.append(container);
  const root = createRoot(container);
  const render = (sessionId: string, refreshToken = 0) =>
    flushSync(() =>
      root.render(
        <StrictMode>
          <BrowserProductTheme>
            <SurfaceProvider>
              <ReviewFileTree
                key={sessionId}
                sessionId={sessionId}
                cwd="/fixture"
                currentPath={undefined}
                onOpenFile={noop}
                onClose={noop}
                refreshToken={refreshToken}
              />
            </SurfaceProvider>
          </BrowserProductTheme>
        </StrictMode>,
      )
    );
  const text = () => container.textContent ?? "";
  const folder = (path: string) => {
    const row = container.querySelector<HTMLElement>(
      `[data-code-tree-path="${path}"]`,
    );
    check(row, `missing folder ${path}`);
    return row;
  };
  const toggle = (path: string) => flushSync(() => folder(path).click());
  const type = (value: string) => {
    const input = container.querySelector("input");
    check(input, "missing file search");
    flushSync(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!
        .set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const expand = async () => {
    await until(
      () => !!container.querySelector('[data-code-tree-path="src"]'),
      "root",
    );
    toggle("src");
    await until(() => text().includes("nested"), "child directory");
    toggle("src/nested");
    await until(() => text().includes(`file-${version}.ts`), "nested file");
  };
  try {
    render("tree-refresh");
    await expand();
    const expandedRow = folder("src/nested");
    version = 2;
    render("tree-refresh", 1);
    await until(
      () => text().includes("file-2.ts"),
      "refresh of an expanded nested folder",
    );
    check(
      !text().includes("file-1.ts"),
      "refresh retained a removed nested file",
    );
    check(
      folder("src/nested").getAttribute("aria-expanded") === "true",
      "refresh collapsed the current folder",
    );
    check(
      folder("src/nested") === expandedRow,
      "refresh replaced the existing folder row",
    );
    tests.push(
      "refresh updates expanded nested folders while preserving their expansion",
    );

    type("query");
    await until(() => text().includes("query-2.ts"), "initial search");
    version = 3;
    render("tree-refresh", 2);
    await until(
      () => text().includes("query-3.ts"),
      "search after workspace refresh",
    );
    check(
      !text().includes("query-2.ts"),
      "refresh retained stale search results",
    );
    tests.push("workspace refresh reruns the active file search");

    hold = (url) =>
      url.pathname.endsWith("/search") && url.searchParams.get("q") === "old";
    type("old");
    await until(() => pending.length > 0, "old search");
    const oldSearch = pending.shift()!;
    type("current");
    await until(() => text().includes("current-3.ts"), "current search");
    oldSearch.resolve(response(search("old", 1)));
    await settle();
    check(oldSearch.signal?.aborted, "superseded search was not cancelled");
    check(
      text().includes("current-3.ts") && !text().includes("old-1.ts"),
      "a cancelled search replaced the current results",
    );
    type("old");
    await until(() => pending.length > 0, "old failing search");
    const oldFailure = pending.shift()!;
    type("current");
    await until(
      () => text().includes("current-3.ts"),
      "current search before stale failure",
    );
    oldFailure.resolve(new Response("gone", { status: 404 }));
    await settle();
    check(
      !text().includes("File search is unavailable"),
      "a cancelled search installed a stale error",
    );
    tests.push(
      "superseded file searches cannot overwrite results or install errors",
    );

    hold = (url) => url.searchParams.get("path") === "src/nested";
    render("tree-prefetch");
    await until(
      () => !!container.querySelector('[data-code-tree-path="src"]'),
      "prefetch root",
    );
    toggle("src");
    await until(() => pending.length > 0, "background child prefetch");
    const prefetch = pending.splice(0);
    hold = () => false;
    toggle("src/nested");
    await until(() => text().includes("file-3.ts"), "explicit folder load");
    for (const read of prefetch) read.resolve(response(page("src/nested", 1)));
    await settle();
    check(
      prefetch.every((read) => read.signal?.aborted),
      "explicit load did not cancel its prefetch",
    );
    toggle("src/nested");
    toggle("src/nested");
    await settle();
    check(
      text().includes("file-3.ts") && !text().includes("file-1.ts"),
      "cancelled prefetch poisoned the folder cache",
    );
    tests.push(
      "cancelled directory prefetch cannot replace the explicit load in the memory cache",
    );

    toggle("src/nested");
    hold = (url) => url.searchParams.get("path") === "src/nested";
    render("tree-prefetch", 1);
    await until(() => pending.length > 0, "held child prefetch");
    toggle("src/nested");
    const stale = pending.splice(0);
    check(
      stale.some((read) => !read.signal?.aborted),
      "missing pending explicit folder read",
    );
    hold = () => false;
    version = 4;
    render("tree-prefetch", 2);
    await until(() => text().includes("file-4.ts"), "folder after refresh");
    for (const read of stale) read.resolve(response(page("src/nested", 1)));
    await settle();
    check(
      stale.every((read) => read.signal?.aborted),
      "refresh retained an older directory request",
    );
    check(
      text().includes("file-4.ts") && !text().includes("file-1.ts"),
      "old directory response overwrote the refreshed tree",
    );
    tests.push(
      "refresh cancels old directory reads and rejects their late replies",
    );

    hold = (url) => url.pathname.includes("tree-return") && !url.search;
    render("tree-return");
    await until(() => pending.length > 0, "root before leaving workspace");
    const abandoned = pending.splice(0);
    render("tree-other");
    await until(
      () => !!container.querySelector('[data-code-tree-path="src"]'),
      "other workspace root",
    );
    for (const read of abandoned) {
      read.resolve(response({ ...page("", 1), entries: [] }));
    }
    await settle();
    check(
      abandoned.every((read) => read.signal?.aborted),
      "leaving retained a pending root request",
    );
    hold = () => false;
    render("tree-return");
    await until(
      () => !!container.querySelector('[data-code-tree-path="src"]'),
      "return after the abandoned root reply",
    );
    tests.push(
      "leaving a workspace prevents late root replies from poisoning its cache on return",
    );
  } finally {
    flushSync(() => root.unmount());
    for (const read of pending) {
      read.resolve(new Response(null, { status: 404 }));
    }
    container.remove();
    globalThis.fetch = originalFetch;
  }
  return tests;
}
