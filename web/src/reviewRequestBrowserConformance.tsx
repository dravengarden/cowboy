/** Out-of-order Git reads against the real components, without a live account. */
import { type ReactNode, StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { ThemeProvider } from "@mui/material";
import { useThemeMode } from "./theme.ts";
import { COARSE_POINTER_ROOT_CLASS } from "./platform.ts";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";
import { ReviewChanges } from "./mobile/review/ReviewChanges.tsx";
import { ReviewRepository } from "./mobile/review/ReviewRepository.tsx";
import type { GitReviewEntry } from "./mobile/review/gitReviewModel.ts";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(predicate: () => boolean, label: string) {
  for (let n = 0; n < 500; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`Git request fixture timed out: ${label}`);
}
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 50));
const noop = () => undefined;
const response = (value: unknown) =>
  new Response(JSON.stringify(value), {
    headers: { "Content-Type": "application/json" },
  });
const changes = (name: string, comparison: string) => ({
  apiVersion: 1,
  revision: name,
  head: "abcdef0",
  truncated: false,
  comparison,
  comparisons: ["main", "next", "pending"].map((label) => ({
    reference: `refs/heads/${label}`,
    label,
  })),
  changes: [{
    path: `${name}.ts`,
    status: "modified",
    staged: false,
    unstaged: true,
  }],
});
const history = (subject: string, truncated: boolean) => ({
  apiVersion: 1,
  commits: [{
    oid: subject,
    parents: [],
    author: "Fixture",
    authoredAt: "2026-09-30T00:00:00Z",
    subject,
    decorations: [],
  }],
  historyTruncated: truncated,
  worktrees: [],
});

export async function runReviewRequestBrowserConformance(): Promise<string[]> {
  const tests: string[] = [];
  const originalFetch = globalThis.fetch;
  const pending: Array<{
    url: URL;
    signal: AbortSignal | null | undefined;
    resolve: (value: Response) => void;
  }> = [];
  let fail = true;
  let hold = false;
  let subject = "initial-history";
  let historyTruncated = true;
  globalThis.fetch = (input, init) => {
    const url = new URL(String(input), location.href);
    if (
      url.pathname.endsWith("/repository") && !url.searchParams.has("after")
    ) {
      return Promise.resolve(response(history(subject, historyTruncated)));
    }
    if (hold || url.searchParams.has("after")) {
      // Deliberately ignore abort: cancellation alone cannot fence a parsed reply.
      return new Promise((resolve) =>
        pending.push({ url, signal: init?.signal, resolve })
      );
    }
    if (fail) return Promise.resolve(new Response("gone", { status: 404 }));
    const comparison = url.searchParams.get("comparison") ?? "";
    return Promise.resolve(
      response(changes(comparison.split("/").at(-1)!, comparison)),
    );
  };
  const container = document.createElement("div");
  container.style.cssText =
    "width:420px;height:600px;display:flex;flex-direction:column";
  document.body.append(container);
  const root = createRoot(container);
  let setMode: ReturnType<typeof useThemeMode>["setMode"];
  function ProductTheme({ children }: { children: ReactNode }) {
    const controls = useThemeMode();
    setMode = controls.setMode;
    return <ThemeProvider theme={controls.theme}>{children}</ThemeProvider>;
  }
  const revisions: string[] = [];
  const queues: GitReviewEntry[][] = [];
  const onRevision = (revision: string) => {
    revisions.push(revision);
  };
  const onChanges = (queue: GitReviewEntry[]) => {
    queues.push(queue);
  };
  const render = (comparison: string, repository = false, refreshToken = 0) =>
    flushSync(() =>
      root.render(
        <StrictMode>
          <ProductTheme>
            <SurfaceProvider>
              {repository
                ? (
                  <ReviewRepository
                    sessionId="fixture"
                    onOpenDiff={noop}
                    onOpenCommit={noop}
                    reviewed={new Set()}
                    onRevision={noop}
                    comparison={null}
                    onComparisonChange={noop}
                    onChanges={noop}
                    onClose={noop}
                    refreshToken={refreshToken}
                  />
                )
                : (
                  <ReviewChanges
                    sessionId="fixture"
                    comparison={comparison}
                    onComparisonChange={noop}
                    onOpenDiff={noop}
                    reviewed={new Set()}
                    onRevision={onRevision}
                    onChanges={onChanges}
                  />
                )}
            </SurfaceProvider>
          </ProductTheme>
        </StrictMode>,
      )
    );
  const text = () => container.textContent ?? "";
  const click = (label: string) => {
    const button = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((node) =>
        node.getAttribute("aria-label") === label ||
        node.textContent?.trim() === label
      );
    check(button && !button.disabled, `missing enabled ${label}`);
    button.click();
  };
  try {
    render("refs/heads/main");
    await until(
      () => text().includes("Git changes are unavailable"),
      "initial error",
    );
    fail = false;
    hold = true;
    click("Retry Git changes");
    await until(() => pending.length === 1, "manual retry");
    const retry = pending.shift()!;
    hold = false;
    render("refs/heads/next");
    await until(() => text().includes("next.ts"), "new comparison");
    const settledRevisions = revisions.length;
    retry.resolve(response(changes("stale-retry", "refs/heads/main")));
    await settle();
    check(
      text().includes("next.ts") && !text().includes("stale-retry.ts"),
      "a retry from the old branch overwrote the new comparison",
    );
    check(
      revisions.length === settledRevisions,
      "a stale read published its revision",
    );
    check(
      retry.signal?.aborted,
      "switching comparison did not cancel the manual retry",
    );
    tests.push(
      "manual retry responses cannot overwrite a later branch comparison or its revision",
    );

    hold = true;
    const queueStart = queues.length;
    render("refs/heads/pending");
    await until(() => pending.length === 1, "pending comparison");
    check(
      !queues.slice(queueStart).some((queue) =>
        queue.some((entry) =>
          entry.change.path === "next.ts" &&
          entry.comparison === "refs/heads/pending"
        )
      ),
      "old files were relabeled with the newly selected comparison",
    );
    pending.shift()!.resolve(
      response(changes("pending", "refs/heads/pending")),
    );
    await until(() => text().includes("pending.ts"), "pending files");
    tests.push(
      "changing a comparison clears the old diff navigation queue before its new response arrives",
    );

    render("refs/heads/main");
    await until(() => pending.length === 1, "superseded failure");
    const failedRead = pending.shift()!;
    hold = false;
    render("refs/heads/next");
    await until(
      () => text().includes("next.ts"),
      "comparison before stale failure",
    );
    failedRead.resolve(new Response("unavailable", { status: 502 }));
    await settle();
    check(
      !text().includes("Git changes are unavailable"),
      "a cancelled read installed an error on the new comparison",
    );
    tests.push(
      "a cancelled branch read cannot replace current content with its late failure",
    );

    historyTruncated = false;
    render("", true);
    await settle();
    for (const mode of ["light", "dark"] as const) {
      flushSync(() => setMode(mode));
      await settle();
      document.documentElement.classList.add(COARSE_POINTER_ROOT_CLASS);
      for (const label of ["Changes", "History", "Worktrees", "History"]) {
        const tab = [...container.querySelectorAll<HTMLElement>('[role="tab"]')]
          .find((node) => node.textContent?.trim() === label)!;
        flushSync(() => {
          tab.dispatchEvent(
            new PointerEvent("pointerdown", {
              bubbles: true,
              pointerType: "touch",
            }),
          );
          tab.click();
        });
        tab.classList.add("Mui-focusVisible");
        await new Promise<void>((resolve) => setTimeout(resolve, 200));
        const background = getComputedStyle(tab).backgroundColor;
        check(
          tab.getAttribute("aria-selected") === "true" &&
            background !== "transparent" && background !== "rgba(0, 0, 0, 0)",
          `${mode}: selected touch tab lost its fill`,
        );
        for (
          const other of container.querySelectorAll<HTMLElement>(
            '[role="tab"][aria-selected="false"]',
          )
        ) {
          other.dataset.touchActivated = "true";
          other.classList.add("Mui-focusVisible");
          // MUI interpolates the previous tab's background for 150ms.
          await new Promise<void>((resolve) => setTimeout(resolve, 200));
          const fill = getComputedStyle(other).backgroundColor;
          check(
            fill === "transparent" || /rgba\([^)]*,\s*0\)$/.test(fill),
            `${mode}: old touch tab retained selected paint (${fill})`,
          );
        }
      }
    }
    document.documentElement.classList.remove(COARSE_POINTER_ROOT_CLASS);
    tests.push(
      "real light and dark themes retain selected tab fill after touch and clear the old tab",
    );
    render("refs/heads/next");
    historyTruncated = true;
    render("", true);
    click("History");
    await until(
      () => pending.some((read) => read.url.searchParams.has("after")),
      "older history page",
    );
    const older = pending.shift()!;
    check(
      older.url.searchParams.get("after") === "initial-history",
      "history omitted its original cursor",
    );
    subject = "refreshed-history";
    historyTruncated = false;
    render("", true, 1);
    await until(
      () => text().includes("refreshed-history"),
      "refreshed history",
    );
    older.resolve(response(history("stale-older-history", false)));
    await settle();
    check(
      text().includes("refreshed-history") &&
        !text().includes("stale-older-history"),
      "an older page replaced the refreshed history",
    );
    check(older.signal?.aborted, "refresh did not cancel the old page request");

    subject = "unmount-history";
    historyTruncated = true;
    render("", true, 2);
    await until(() => pending.length === 1, "page before unmount");
    const unmounted = pending.shift()!;
    render("refs/heads/next");
    check(
      unmounted.signal?.aborted,
      "leaving the repository retained its pending page request",
    );
    unmounted.resolve(response(history("unmounted-history", false)));
    await settle();
    tests.push(
      "history paging retains its cursor and is cancelled across refresh and repository teardown",
    );
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    document.documentElement.classList.remove(COARSE_POINTER_ROOT_CLASS);
    globalThis.fetch = originalFetch;
  }
  return tests;
}
