/** Actual Review DocumentView refresh behaviour. Synthetic HTTP, no account. */
import { StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";
import { DocumentView } from "./mobile/review/ReviewApp.tsx";
import { DOCUMENT_REFRESH_RESUME_GRACE_MS } from "./mobile/review/documentRefreshModel.ts";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(predicate: () => boolean, label: string) {
  for (let n = 0; n < 500; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`Review refresh fixture timed out: ${label}`);
}
const frames = () =>
  new Promise<void>((resolve) =>
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
  );
const sleep = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));

function handbook(inserted: number): string {
  return [
    "# Handbook",
    ...Array.from(
      { length: inserted },
      (_, index) => `## Inserted ${index}\n\nNew material ${index}.\n`,
    ),
    ...Array.from(
      { length: 80 },
      (_, index) =>
        `## Section ${index}\n\nParagraph ${index} explains one stable part of the handbook.\n`,
    ),
  ].join("\n");
}

const noop = () => undefined;
const noLink = () => false;
const TARGET = { kind: "source", path: "notes.md" } as const;

export async function runReviewDocumentRefreshBrowserConformance(): Promise<
  string[]
> {
  const tests: string[] = [];
  let served = { revision: "r1", text: handbook(0) };
  const fileRequests: number[] = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (input: RequestInfo | URL) => {
    const url = new URL(String(input), location.href);
    if (url.pathname === "/api/code/sessions/workspace%3A%3Afixture/file") {
      fileRequests.push(Date.now());
      return Promise.resolve(
        new Response(
          JSON.stringify({
            apiVersion: 1,
            path: TARGET.path,
            revision: served.revision,
            text: served.text,
            truncated: false,
            size: served.text.length,
          }),
          { headers: { "Content-Type": "application/json" } },
        ),
      );
    }
    return Promise.resolve(new Response("not found", { status: 404 }));
  };
  const container = document.createElement("div");
  container.style.cssText =
    "width: 420px; height: 600px; display: flex; flex-direction: column;";
  document.body.append(container);
  const root = createRoot(container);
  const positions = new Map<string, number>();
  const readScrollPosition = (key: string) => positions.get(key);
  const rememberScrollPosition = (key: string, top: number) => {
    positions.set(key, top);
  };
  const render = (dataRevision: number, active: boolean) =>
    flushSync(() =>
      root.render(
        <StrictMode>
          <ThemeProvider theme={createTheme()}>
            <SurfaceProvider>
              <DocumentView
                sessionId="workspace::fixture"
                target={TARGET}
                onRevision={noop}
                markdownPreview
                onNavigate={noop}
                closeSymbolRequest={0}
                onRestoreSymbolConsumed={noop}
                onSymbolOpenChange={noop}
                onVisibleSourceLine={noop}
                onBufferUnavailable={noop}
                readScrollPosition={readScrollPosition}
                rememberScrollPosition={rememberScrollPosition}
                onMarkdownLink={noLink}
                bufferMode="legacy"
                dataRevision={dataRevision}
                active={active}
                outlineOpen={false}
                onOutlineClose={noop}
                outlineLine={undefined}
                onOutlineSelect={noop}
              />
            </SurfaceProvider>
          </ThemeProvider>
        </StrictMode>,
      )
    );
  const article = () =>
    container.querySelector<HTMLElement>("[data-markdown-review-preview]");
  const scroller = () =>
    container.querySelector<HTMLElement>(
      "[data-mobile-overflow-layer='true']",
    );
  const pill = () => container.querySelector("[role='status']")?.textContent;
  const heading = (label: string) =>
    [...container.querySelectorAll("h2")].find((element) =>
      element.textContent === label
    );
  const headingOffset = (label: string) => {
    const element = heading(label);
    const view = scroller();
    check(element && view, `missing ${label}`);
    return element.getBoundingClientRect().top -
      view.getBoundingClientRect().top;
  };
  const assertAnchored = (expected: number, label: string) => {
    const offset = headingOffset("Section 40");
    check(
      Math.abs(offset - expected) <= 2,
      `${label}: anchor moved from ${expected} to ${offset}`,
    );
  };

  try {
    render(0, true);
    await until(() => Boolean(heading("Section 79")), "initial document");
    await frames();
    const view = scroller();
    check(view, "missing preview scroller");
    view.scrollTop += headingOffset("Section 40") - 24;
    await frames();
    const anchored = headingOffset("Section 40");
    const original = article();
    // A change detected right after opening is unseen; wait out that grace.
    await sleep(DOCUMENT_REFRESH_RESUME_GRACE_MS + 150);

    const top = view.scrollTop;
    const baseline = fileRequests.length;
    render(1, true);
    await until(
      () => fileRequests.length === baseline + 1,
      "unrelated revalidation",
    );
    await sleep(50);
    await frames();
    check(article() === original, "unrelated change remounted the preview");
    check(!container.querySelector("[role='progressbar']"), "spinner shown");
    check(view.scrollTop === top, "unrelated change moved the reader");
    check(pill() === undefined, "unrelated change prompted");
    tests.push(
      "a worktree change that leaves the open document's revision unchanged keeps the same preview, scroll and no prompt",
    );

    served = { revision: "r2", text: handbook(3) };
    render(2, true);
    await until(() => pill()?.includes("This file changed") ?? false, "prompt");
    check(!heading("Inserted 0"), "reading view changed without consent");
    check(article() === original, "prompt remounted the preview");
    assertAnchored(anchored, "prompt");
    tests.push(
      "a changed document being read keeps its current text under a refresh prompt",
    );

    const refresh = [...container.querySelectorAll("button")].find((button) =>
      button.textContent?.includes("Refresh")
    );
    check(refresh, "missing Refresh");
    flushSync(() => refresh.click());
    await until(() => Boolean(heading("Inserted 2")), "manual refresh");
    await frames();
    check(pill() === undefined, "prompt survived refresh");
    check(article() === original, "manual refresh remounted the preview");
    assertAnchored(anchored, "manual refresh");
    tests.push(
      "Refresh applies in place and keeps the top visible block where the reader left it despite content inserted above",
    );

    render(2, false);
    served = { revision: "r3", text: handbook(6) };
    render(3, false);
    await until(() => Boolean(heading("Inserted 5")), "background apply");
    await frames();
    check(pill() === undefined, "background change prompted");
    assertAnchored(anchored, "background apply");
    tests.push(
      "a change while Review is in the background applies automatically and keeps the reading position",
    );

    render(3, true);
    await sleep(DOCUMENT_REFRESH_RESUME_GRACE_MS + 150);
    served = { revision: "r4", text: handbook(8) };
    render(4, true);
    await until(
      () => pill()?.includes("This file changed") ?? false,
      "second prompt",
    );
    render(4, false);
    await until(() => Boolean(heading("Inserted 7")), "apply on leave");
    await frames();
    check(pill() === undefined, "prompt survived leaving Review");
    assertAnchored(anchored, "apply on leave");
    tests.push(
      "leaving Review applies a pending change so the reader returns to current text at the same place",
    );
    return tests;
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    globalThis.fetch = originalFetch;
  }
}
