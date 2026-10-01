/** Real mobile PR reader with synthetic HTTP; no account or repository access. */
import { StrictMode, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { SurfaceProvider } from "../../surface/SurfaceProfile";
import { RemoteReviewApp } from "./RemoteReviewApp";
import type { RemoteReviewBinding } from "./remoteReviewModel";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(label: string, predicate: () => boolean) {
  for (let n = 0; n < 300; n++) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`PR fixture timed out: ${label}`);
}

export async function runRemoteReviewBrowserConformance(): Promise<string[]> {
  const originalFetch = globalThis.fetch;
  const requests: URL[] = [];
  const pending: Array<(response: Response) => void> = [];
  let saved: RemoteReviewBinding | null = null;
  let local = false;
  let delayList = false;
  const listPending: Array<(response: Response) => void> = [];
  const json = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { "content-type": "application/json" },
    });
  const review = (title = "Remote change") => ({
    repositoryId: "123",
    number: "12",
    title,
    url: "https://github.com/owner/repo/pull/12",
    state: "open",
    head: "a".repeat(40),
    base: "b".repeat(40),
    revision: "c".repeat(64),
    totalFiles: 21,
    nextPage: 2,
    limited: false,
    files: [{
      path: "review.txt",
      oldPath: null,
      status: "modified",
      additions: 1,
      deletions: 1,
      patch: "@@ -1 +1 @@\n-local\n+remote",
      limited: false,
    }],
  });
  const pulls = (title = "Remote change", empty = false) =>
    json({
      type: "pulls",
      account: "owner",
      total: empty ? 0 : 1,
      incomplete: false,
      nextPage: null,
      items: empty ? [] : [{
        number: "12",
        title,
        repository: "owner/repo",
        url: "https://github.com/owner/repo/pull/12",
        author: "owner",
        state: "open",
        draft: false,
        updatedAt: "2026-10-01T00:00:00Z",
      }],
    });
  globalThis.fetch = (input, init) => {
    check(init?.cache === "no-store", "private PR reads must bypass caches");
    const url = new URL(String(input), location.href);
    requests.push(url);
    if (url.pathname.endsWith("/extensions")) {
      return Promise.resolve(json({
        type: "inventory",
        remotes: [{
          name: "origin",
          host: "github.com",
          owner: "workspace",
          repository: "local",
        }],
        extensions: [{
          identity: {
            pluginId: "fixture-review",
            pluginVersion: "0.2.0",
            generationDigest: `sha256:${"1".repeat(64)}`,
          },
          label: "Review fixture",
          description: "Fixture",
          available: true,
          views: [{
            id: "pulls",
            label: "PRs",
            filters: [],
            review: "pull_request",
            discovery: true,
          }],
        }],
      }));
    }
    if (url.searchParams.get("discovery") === "true") {
      if (delayList) return new Promise((resolve) => listPending.push(resolve));
      return Promise.resolve(pulls());
    }
    check(
      url.searchParams.get("review") === "true",
      "PR reader used a workspace file route",
    );
    if (url.searchParams.get("page") === "2") {
      return new Promise((resolve) => pending.push(resolve));
    }
    return Promise.resolve(json({ type: "review", review: review() }));
  };
  const container = document.createElement("div");
  container.style.cssText = "width:390px;height:740px";
  document.body.append(container);
  const root = createRoot(container);
  function Harness({ context }: { context: string }) {
    const [binding, setBinding] = useState<RemoteReviewBinding | null>(null);
    return (
      <RemoteReviewApp
        context={context}
        title={context}
        binding={binding}
        active
        onBind={(value) => {
          saved = value;
          setBinding(value);
        }}
        onLocal={() => {
          local = true;
        }}
      />
    );
  }
  function render(context: string) {
    flushSync(() =>
      root.render(
        <StrictMode>
          <ThemeProvider theme={createTheme()}>
            <SurfaceProvider>
              <Harness key={context} context={context} />
            </SurfaceProvider>
          </ThemeProvider>
        </StrictMode>,
      )
    );
  }
  function click(label: string) {
    const button = [
      ...container.querySelectorAll<HTMLElement>("button,[role=button]"),
    ].find((node) =>
      node.getAttribute("aria-label") === label || node.textContent === label
    );
    check(button, `missing ${label}`);
    check(
      !(button instanceof HTMLButtonElement && button.disabled),
      `disabled ${label}`,
    );
    flushSync(() => button.click());
  }
  try {
    render("first");
    await until(
      "default PR list",
      () => !!container.querySelector('[aria-label="Review owner/repo #12"]'),
    );
    const discovery = requests.find((url) =>
      url.searchParams.get("discovery") === "true"
    );
    check(
      discovery?.searchParams.get("relation") === "author" &&
        discovery.searchParams.get("currentRepository") === "false",
      "default is not current account across repositories",
    );
    const back = container.querySelector(
      '[aria-label="Local worktree review"]',
    )!;
    check(
      back.getBoundingClientRect().top >
        container.getBoundingClientRect().top + 500,
      "back action is not thumb reachable",
    );
    delayList = true;
    click("Refresh pull requests");
    await until("refresh pending", () => listPending.length > 0);
    check(
      !!container.querySelector('[aria-label="Review owner/repo #12"]'),
      "refresh discarded readable list",
    );
    listPending.splice(0).forEach((resolve) =>
      resolve(json({ type: "unavailable", code: "request_failed" }))
    );
    await until(
      "refresh error",
      () => !!container.querySelector('[role="alert"]'),
    );
    check(
      !!container.querySelector('[aria-label="Review owner/repo #12"]'),
      "refresh failure discarded list",
    );
    click("Retry");
    await until("retry pending", () => listPending.length > 0);
    listPending.splice(0).forEach((resolve) => resolve(pulls()));
    await until(
      "retry complete",
      () => !container.querySelector('[role="alert"]'),
    );
    delayList = false;
    click("Review owner/repo #12");
    await until(
      "association",
      () => saved !== null && !!container.textContent?.includes("review.txt"),
    );
    check(
      requests.some((url) =>
        url.searchParams.get("review") === "true" &&
        url.searchParams.get("repository") === "owner/repo"
      ),
      "cross-repository selection lost its target",
    );
    check(
      (saved as RemoteReviewBinding | null)?.repositoryId === "123",
      "association missing immutable repository identity",
    );
    check(
      !JSON.stringify(saved).includes("patch"),
      "association persisted private code",
    );
    const file = [...container.querySelectorAll<HTMLElement>("[role=button]")]
      .find((node) => node.textContent?.includes("review.txt"));
    check(file, "missing changed file");
    flushSync(() => file.click());
    await until("CodeMirror", () => !!container.querySelector(".cm-editor"));
    check(
      container.textContent?.includes("remote"),
      "remote patch did not render",
    );
    check(
      requests.every((url) => url.pathname.includes("/extensions")),
      "remote diff contacted local file or native buffer API",
    );
    click("PR files");
    click("Next PR files");
    await until("page pending", () => pending.length > 0);
    const last = requests.at(-1)!;
    check(
      last.searchParams.get("repositoryId") === "123" &&
        last.searchParams.get("revision") === "c".repeat(64),
      "continuation lost snapshot identity",
    );
    pending.splice(0).forEach((resolve) =>
      resolve(json({ type: "unavailable", code: "review_changed" }))
    );
    await until(
      "changed refusal",
      () => !!container.textContent?.includes("changed while loading"),
    );
    check(
      container.textContent?.includes("review.txt"),
      "failed continuation discarded the readable page",
    );
    click("Next PR files");
    await until("late page", () => pending.length > 0);
    render("second");
    pending.splice(0).forEach((resolve) =>
      resolve(json({ type: "review", review: review("STALE PRIVATE PR") }))
    );
    await until(
      "new session",
      () => !!container.textContent?.includes("Choose a pull request"),
    );
    await new Promise((resolve) => setTimeout(resolve, 30));
    check(
      !container.textContent?.includes("STALE PRIVATE PR"),
      "old session response leaked into new session",
    );
    delayList = true;
    render("slow");
    await until(
      "skeleton",
      () =>
        !!container.querySelector('[aria-label="Loading pull requests"]') &&
        listPending.length > 0,
    );
    click("Local worktree review");
    check(local, "loading blocked Back");
    local = false;
    const abandoned = listPending.splice(0);
    render("new-list");
    await until("new list pending", () => listPending.length > 0);
    abandoned.forEach((resolve) => resolve(pulls("PRIVATE OLD LIST")));
    listPending.splice(0).forEach((resolve) => resolve(pulls("", true)));
    await until(
      "empty state",
      () => !!container.textContent?.includes("No matching pull requests"),
    );
    check(
      !container.textContent?.includes("PRIVATE OLD LIST"),
      "abandoned discovery leaked across sessions",
    );
    click("Change filters");
    check(
      !!container.textContent?.includes("Relationship") &&
        !!container.textContent?.includes("Status"),
      "empty state cannot reach filters",
    );
    delayList = false;
    const relationship = container.querySelector<HTMLElement>(
      '[role="combobox"]',
    );
    check(relationship, "missing relationship filter");
    flushSync(() =>
      relationship.dispatchEvent(
        new MouseEvent("mousedown", { bubbles: true, button: 0 }),
      )
    );
    await until(
      "relationship options",
      () => !!document.querySelector('[role="option"][data-value="review"]'),
    );
    flushSync(() =>
      (document.querySelector(
        '[role="option"][data-value="review"]',
      ) as HTMLElement).click()
    );
    await until(
      "review filter read",
      () =>
        requests.some((url) => url.searchParams.get("relation") === "review"),
    );
    await until(
      "filtered results",
      () => !!container.querySelector('[aria-label="Review owner/repo #12"]'),
    );
    click("Done");
    click("Local worktree review");
    check(local, "local source was unreachable");
    return [
      "account PR discovery is the default and Back stays at the bottom",
      "refresh and failure preserve readable PRs with explicit retry",
      "slow discovery shows skeletons and retains working bottom navigation",
      "abandoned account lists cannot cross sessions and empty lists offer filters",
      "relationship filter executes a new bounded account search",
      "PR association retains repository identity without code bodies",
      "actual CodeMirror renders the remote patch without workspace or native reads",
      "PR page continuation retains repository and revision",
      "concurrent remote update refuses continuation and preserves readable files",
      "late private PR replies cannot cross session changes",
      "local worktree remains reachable from remote review",
    ];
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    globalThis.fetch = originalFetch;
  }
}
