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
          owner: "owner",
          repository: "repo",
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
          }],
        }],
      }));
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
      "repository",
      () => !!container.querySelector('input:not([aria-hidden="true"])'),
    );
    const input = container.querySelector<HTMLInputElement>(
      'input:not([aria-hidden="true"])',
    )!;
    flushSync(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!
        .call(input, "https://github.com/owner/repo/pull/12");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await until(
      "associate enabled",
      () =>
        [...container.querySelectorAll("button")].some((button) =>
          button.textContent === "Associate and review" && !button.disabled
        ),
    );
    click("Associate and review");
    await until(
      "association",
      () => saved !== null && !!container.textContent?.includes("review.txt"),
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
      () => !!container.textContent?.includes("Associate this session"),
    );
    await new Promise((resolve) => setTimeout(resolve, 30));
    check(
      !container.textContent?.includes("STALE PRIVATE PR"),
      "old session response leaked into new session",
    );
    click("Local worktree review");
    check(local, "local source was unreachable");
    return [
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
