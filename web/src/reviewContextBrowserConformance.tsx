/** Real Review app and store. Isolated Machine inventories and HTTP, no account. */
import { CssBaseline } from "@mui/material";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { ReviewApp } from "./mobile/review/ReviewApp";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { getActiveSessionId, setActiveSessionId } from "./controlPlane";
import type { SessionMeta } from "./protocol";

const wait = (ms = 50) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(predicate: () => boolean, message: string) {
  for (let i = 0; i < 250; i++) {
    if (predicate()) return;
    await wait(20);
  }
  throw new Error(`${message}: ${document.body.textContent?.slice(0, 1500)}`);
}

export function renderReviewContextPreview(dense = false) {
  const originalFetch = globalThis.fetch;
  const originalSocket = globalThis.WebSocket;
  const sessions: SessionMeta[] = [
    {
      id: "worktree",
      cwd: "/runtime/review",
      workspace_id: "cowboy",
      workspace_name: "Cowboy",
      workspace_source_path: "/projects/cowboy",
      machine_id: "ovh",
      execution_binding: {
        schema: 1,
        runtime: { machine_id: "ovh", cwd: "/runtime/review" },
        environment: { machine_id: "hawk", protocol: 1 },
        workspace: { cwd: "/tasks/cowboy/feature" },
      },
      title: "Improve review",
      provider: "codex",
      status: "exited",
    },
    {
      id: "project",
      cwd: "/projects/suger",
      workspace_id: "suger",
      workspace_name: "Suger",
      workspace_source_path: "/projects/suger",
      machine_id: "hawk",
      title: "Suger project",
      provider: "codex",
      status: "exited",
    },
    {
      id: "fallback",
      cwd: "",
      workspace_id: "cowboy",
      workspace_name: "Cowboy",
      machine_id: "hawk",
      title: "Project association",
      provider: "codex",
      status: "exited",
    },
  ];
  const machine = {
    id: "hawk",
    display_name: "Hawk",
    platform: "linux",
    architecture: "x86_64",
    status: "online",
    local: true,
    connected: true,
    schedulable: true,
    capacity: { max_sessions: 20, draining: false },
    active_sessions: 0,
    components: [],
    workspaces: [
      {
        id: "cowboy",
        display_name: "Cowboy",
        canonical_path: "/projects/cowboy",
      },
      { id: "suger", display_name: "Suger", canonical_path: "/projects/suger" },
      {
        id: "personal",
        display_name: "Cowboy",
        canonical_path: "/personal/cowboy",
      },
    ],
  };
  const requests: string[] = [];
  const commands: string[] = [];
  let socket: FixtureSocket | undefined;
  class FixtureSocket {
    static OPEN = 1;
    static CONNECTING = 0;
    static CLOSED = 3;
    readyState = 1;
    protocol = "cowboy-sync-v1";
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: string }) => void) | null = null;
    onclose: (() => void) | null = null;
    onerror = null;
    constructor() {
      socket = this;
      setTimeout(() => {
        this.onopen?.();
        this.publish({
          type: "machines",
          revision: 1,
          machines: [machine, {
            ...machine,
            id: "ovh",
            display_name: "OVH",
            local: false,
            workspaces: [{
              id: "cowboy",
              display_name: "Cowboy",
              canonical_path: "/runtime/cowboy",
            }],
          }],
        });
        this.publish({ type: "sessions", sessions });
        this.publish({ type: "bootstrap_complete" });
      }, 20);
    }
    publish(message: unknown) {
      this.onmessage?.({ data: JSON.stringify(message) });
    }
    send(data: string) {
      commands.push(JSON.parse(data).type);
    }
    close() {
      this.readyState = 3;
    }
  }
  globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;
  globalThis.fetch = (async (input, init) => {
    const url = new URL(String(input), location.href);
    if (!url.pathname.startsWith("/api/")) return originalFetch(input, init);
    requests.push(
      `${init?.method ?? "GET"} ${
        decodeURIComponent(url.pathname)
      }${url.search}`,
    );
    const json = (value: unknown) => Response.json(value);
    if (url.pathname === "/api/sync/dataset") {
      return json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"b".repeat(64)}`,
        user_id: "review-fixture",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      });
    }
    if (url.pathname === "/api/plugins") {
      return json({
        providers: [],
        authentications: [],
        authentication_executors: [],
      });
    }
    if (url.pathname.endsWith("/bootstrap")) return json({ messages: [] });
    if (url.pathname.startsWith("/api/code/sessions/")) {
      const context = decodeURIComponent(url.pathname.split("/")[4]!);
      const clean = context === "project";
      const branch = context === "worktree" ? "feature/review" : "main";
      const path = url.searchParams.get("path") ?? "";
      if (url.pathname.endsWith("/manifest")) {
        return json({
          apiVersion: 1,
          revision: context,
          project: "generated-task-directory",
          branch,
          worktree: context === "worktree"
            ? "/tasks/cowboy/feature"
            : "/projects/cowboy",
          changeCount: clean ? 0 : 1,
          bufferMode: "unavailable",
          language: {
            provider: "none",
            state: "unavailable",
            diagnostics: false,
            hover: false,
            navigation: false,
            outline: false,
            inlayHints: false,
            semanticTokens: false,
          },
        });
      }
      if (url.pathname.endsWith("/changes")) {
        return json({
          apiVersion: 1,
          revision: context,
          head: "abcdef01",
          truncated: false,
          comparisons: [],
          changes: clean ? [] : [
            {
              path: "src/app.ts",
              status: "modified",
              staged: false,
              unstaged: true,
            },
            ...(dense
              ? Array.from({ length: 50 }, (_, index) => ({
                path: `src/file-${index}.ts`,
                status: "modified",
                staged: false,
                unstaged: true,
              }))
              : []),
          ],
        });
      }
      if (url.pathname.endsWith("/repository")) {
        return json({
          apiVersion: 1,
          commits: [{
            oid: "abcdef01",
            parents: [],
            subject: "Improve review",
            author: "Fixture",
            authoredAt: "2026-10-17T00:00:00Z",
            decorations: [],
          }],
          historyTruncated: false,
          worktrees: [{
            path: "/tasks/cowboy/feature",
            branch,
            bare: false,
            detached: false,
            current: true,
          }],
          target: { branch, changeCount: clean ? 0 : 1 },
        });
      }
      if (url.pathname.endsWith("/tree")) {
        return json({
          apiVersion: 1,
          path,
          revision: context,
          truncated: false,
          entries: path
            ? [{
              name: "app.ts",
              path: "src/app.ts",
              kind: "file",
              ignored: false,
            }]
            : [
              { name: "src", path: "src", kind: "directory", ignored: false },
              {
                name: "README.md",
                path: "README.md",
                kind: "file",
                ignored: false,
              },
              ...Array.from({ length: dense ? 35 : 0 }, (_, index) => ({
                name: `file-${index}.ts`,
                path: `file-${index}.ts`,
                kind: "file",
                ignored: false,
              })),
            ],
        });
      }
      if (url.pathname.endsWith("/file")) {
        return json({
          apiVersion: 1,
          path,
          revision: context,
          text: "# Project guide\n\nReview this workspace.",
          truncated: false,
        });
      }
      if (url.pathname.endsWith("/diff")) {
        return json({
          apiVersion: 1,
          path,
          revision: context,
          text:
            "diff --git a/src/app.ts b/src/app.ts\n--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-old\n+new\n",
          truncated: false,
        });
      }
    }
    return new Response("unavailable", { status: 404 });
  }) as typeof fetch;
  bindProductSyncPrincipal("review-fixture");
  setActiveSessionId("worktree");
  const container = document.createElement("div");
  container.style.cssText = "height:100dvh;--cowboy-system-top-clearance:0px";
  document.body.append(container);
  const root = createRoot(container);
  flushSync(() =>
    root.render(
      <BrowserProductTheme>
        <CssBaseline />
        <SurfaceProvider>
          <ReviewApp active onDrawerOpenChange={() => {}} />
        </SurfaceProvider>
      </BrowserProductTheme>,
    )
  );
  return {
    container,
    requests,
    commands,
    refresh: () => socket?.publish({ type: "sessions", sessions }),
    select: (id: string) => flushSync(() => setActiveSessionId(id)),
    cleanup: () => {
      root.unmount();
      container.remove();
      globalThis.fetch = originalFetch;
      globalThis.WebSocket = originalSocket;
    },
  };
}

export async function runReviewContextBrowserConformance(): Promise<string[]> {
  const fixture = renderReviewContextPreview(true);
  const tests: string[] = [];
  const overview = () =>
    fixture.container.querySelector<HTMLElement>(
      '[aria-label="Repository overview"]',
    );
  const files = () =>
    fixture.container.querySelector<HTMLElement>(
      '[aria-label="Workspace files"]',
    );
  const click = async (element: Element | null | undefined) => {
    check(element instanceof HTMLElement, "Missing Review control");
    flushSync(() => element.click());
    await wait(100);
  };
  const labelled = (label: string) =>
    document.querySelector(`[aria-label="${label}"]`);
  const textButton = (root: ParentNode, label: string) =>
    [...root.querySelectorAll("button, [role='button'], [role='tab']")].find((
      element,
    ) => element.textContent?.trim() === label);
  const bottomActions = (label: string, inline: boolean) => {
    const group = labelled(label);
    check(group instanceof HTMLElement, `Missing ${label}`);
    check(
      group.dataset.reviewNavigationActions === (inline ? "inline" : "sidebar"),
      `${label} used the wrong bottom surface`,
    );
    if (inline) {
      check(
        group.closest('[aria-label="Code Review controls"]'),
        "Overview actions should share its existing bottom bar",
      );
    }
    const page = fixture.container.getBoundingClientRect();
    const rect = group.getBoundingClientRect();
    check(
      rect.top >= page.bottom - 100 && rect.bottom <= page.bottom,
      `${label} must stay at the bottom`,
    );
    for (const button of group.querySelectorAll("button")) {
      const hit = button.getBoundingClientRect();
      check(
        hit.width >= 44 && hit.height >= 44,
        "Bottom actions need 44px touch targets",
      );
      check(
        hit.left >= page.left && hit.right <= page.right,
        "Bottom action was clipped",
      );
    }
    return rect.top;
  };
  try {
    await until(
      () => !!overview()?.textContent?.includes("app.ts"),
      "Session changes should be the first screen",
    );
    check(
      fixture.container.textContent?.includes("Worktree · Cowboy"),
      "Header lost registered project identity to a generated worktree name",
    );
    check(
      overview()?.textContent?.includes("/tasks/cowboy/feature"),
      "Review should show the real worktree path",
    );
    check(
      !fixture.requests.some((request) => request.includes("workspace::")),
      "Opening a session redirected to the stable project checkout",
    );
    check(
      !overview()?.textContent?.includes("/runtime/"),
      "Review exposed the Agent runtime instead of its files workspace",
    );
    tests.push(
      "first visit opens the current session worktree with changes and real project identity",
    );

    await wait(800);
    const actionsTop = bottomActions("Repository controls", true);
    const navigator = overview()!.firstElementChild;
    const list = overview()!.querySelector<HTMLElement>(
      "[data-mobile-overflow-layer]",
    );
    check(list, "Missing scrollable changes");
    list.scrollTop = 140;
    const scrollTop = list.scrollTop;
    check(scrollTop > 0, "Changes fixture must overflow");
    check(
      bottomActions("Repository controls", true) === actionsTop,
      "Scrolling changes moved the bottom actions",
    );
    const repositories = fixture.requests.filter((request) =>
      request.endsWith("/repository")
    ).length;
    await click(textButton(overview()!, "app.tsM"));
    await until(
      () => !overview(),
      "Opening a diff should make room for the file",
    );
    check(
      navigator?.isConnected && list?.isConnected,
      "Opening a file remounted navigation",
    );
    await click(labelled("Open Git changes sidebar"));
    await until(
      () => navigator?.closest('[aria-hidden="false"]') !== null,
      "Sidebar should expose the same navigator",
    );
    await wait(400);
    bottomActions("Repository controls", false);
    check(
      list.scrollTop === scrollTop,
      "Moving navigation lost the reading position",
    );
    check(
      fixture.requests.filter((request) => request.endsWith("/repository"))
        .length === repositories,
      "Moving navigation repeated repository loading",
    );
    await click(labelled("Close repository"));
    const activeTab = fixture.container.querySelector<HTMLElement>(
      '[data-review-tab-key][aria-selected="true"]',
    )!;
    // Synthetic pointers cannot acquire native capture. Exercise the real
    // pointerup activation and its click retargeting after the strip unmounts.
    activeTab.setPointerCapture = () => {};
    const pointer = {
      bubbles: true,
      pointerId: 1,
      pointerType: "touch",
      isPrimary: true,
    };
    flushSync(() =>
      activeTab.dispatchEvent(new PointerEvent("pointerdown", pointer))
    );
    flushSync(() =>
      activeTab.dispatchEvent(new PointerEvent("pointerup", pointer))
    );
    await until(
      () => overview()?.firstElementChild === navigator,
      "Return to the original overview",
    );
    const revealedFile = textButton(overview()!, "app.tsM")!;
    flushSync(() =>
      revealedFile.dispatchEvent(
        new PointerEvent("click", { ...pointer, detail: 1 }),
      )
    );
    check(
      overview(),
      "Returning to navigation activated a file through the paired touch click",
    );
    check(
      list.scrollTop === scrollTop,
      "Returning to the overview lost the reading position",
    );
    const changeRequests = fixture.requests.filter((request) =>
      request.endsWith("/changes")
    ).length;
    await click(labelled("Refresh repository"));
    await until(() =>
      fixture.requests.filter((request) => request.endsWith("/changes"))
        .length > changeRequests, "Refresh must revalidate changes too");
    tests.push(
      "bottom actions stay reachable and returning to one preserved navigator rejects retargeted touch clicks",
    );

    // A saved project context is independent from the Agent session.
    await click(
      fixture.container.querySelector('button[aria-label^="Switch session."]'),
    );
    await until(
      () =>
        !!textButton(
          document,
          "Project codeRegistered project checkout/projects/cowboy",
        ),
      "Context opens on the associated project's worktrees",
    );
    await click(
      textButton(
        document,
        "Project codeRegistered project checkout/projects/cowboy",
      ),
    );
    await until(
      () =>
        !!files()?.textContent?.includes("README.md"),
      "Project file tree should open inline",
    );
    check(
      getActiveSessionId() === "worktree",
      "Browsing project code changed the active Agent session",
    );
    check(
      fixture.requests.some((request) =>
        request.includes("workspace::hawk::cowboy")
      ) && !fixture.requests.some((request) =>
        request.includes("workspace::ovh::")
      ),
      "Project selection used the runtime Machine's inventory",
    );
    fixture.refresh();
    await wait(150);
    check(
      fixture.container.textContent?.includes("Project code · Cowboy"),
      "Session refresh stole the chosen review context",
    );
    tests.push(
      "context defaults to the associated project and manual project browsing survives refresh",
    );

    const tree = files()!.firstElementChild;
    const filesActionsTop = bottomActions("File tree controls", true);
    check(
      !files()!.querySelector('[aria-label="Refresh files"]'),
      "Files still has header actions",
    );
    const treeList = files()!.querySelector<HTMLElement>(
      "[data-mobile-overflow-layer]",
    )!;
    treeList.scrollTop = treeList.scrollHeight;
    check(treeList.scrollTop > 0, "Tree fixture must overflow");
    check(
      bottomActions("File tree controls", true) === filesActionsTop,
      "Scrolling files moved the bottom actions",
    );
    check(
      files()!.querySelector('[data-code-tree-path="file-34.ts"]')!
        .getBoundingClientRect().bottom <= filesActionsTop,
      "The last file is hidden behind bottom controls",
    );
    treeList.scrollTop = 0;
    await click(labelled("Expand folders"));
    await until(
      () =>
        !!files()?.querySelector('[data-code-tree-path="src/app.ts"]'),
      "Expanded directory",
    );
    await click(labelled("Collapse folders"));
    check(
      files()!.querySelector('[data-code-tree-path="src"]')?.getAttribute(
        "aria-expanded",
      ) === "false",
      "Bottom collapse action did not collapse the tree",
    );
    await click(labelled("Expand folders"));
    const treeRequests = () =>
      fixture.requests.filter((request) => request.endsWith("/tree")).length;
    const beforeTreeRefresh = treeRequests();
    await click(labelled("Refresh files"));
    await until(
      () => treeRequests() > beforeTreeRefresh,
      "Bottom refresh did not revalidate files",
    );
    const src = files()!.querySelector('[data-code-tree-path="src"]');
    await click(files()!.querySelector('[data-code-tree-path="README.md"]'));
    await until(
      () => !!fixture.container.querySelector("[data-markdown-review-preview]"),
      "Read project file",
    );
    await click(labelled("Open file tree sidebar"));
    await wait(400);
    bottomActions("File tree controls", false);
    check(
      tree?.isConnected && src?.getAttribute("aria-expanded") === "true",
      "Opening source lost the expanded tree",
    );
    const beforeSidebarRefresh = treeRequests();
    await click(labelled("Refresh files"));
    await until(
      () => treeRequests() > beforeSidebarRefresh,
      "Sidebar refresh did not revalidate files",
    );
    treeList.scrollTop = treeList.scrollHeight;
    const sidebarActionsTop = bottomActions("File tree controls", false);
    check(
      tree!.querySelector('[data-code-tree-path="file-34.ts"]')!
        .getBoundingClientRect().bottom <= sidebarActionsTop,
      "The sidebar footer hides the last file",
    );
    await click(labelled("Reveal current file"));
    await until(() => {
      const current = tree!.querySelector('[data-code-tree-path="README.md"]')!
        .getBoundingClientRect();
      return current.top > 0 && current.bottom < sidebarActionsTop;
    }, "Bottom locate action should reveal the current file");
    await click(labelled("Close project code"));
    tests.push(
      "bottom file controls expand, refresh and reveal files without losing tree state or hiding the last row",
    );

    fixture.select("project");
    await until(
      () => !!overview()?.textContent?.includes("Working tree is clean"),
      "A new session should replace the manual project context",
    );
    check(
      fixture.container.textContent?.includes("Project code · Suger"),
      "Shared project was labelled as another session's worktree",
    );
    check(
      !fixture.requests.some((request) =>
        request.includes("/sessions/project/file?")
      ),
      "The previous context's document was read against the new session",
    );
    await click(textButton(overview()!, "Browse files"));
    await until(
      () => !!files()?.textContent?.includes("README.md"),
      "Clean workspace should offer file browsing",
    );
    tests.push(
      "switching sessions restores its own project and a clean workspace has a direct next action",
    );

    fixture.select("fallback");
    await until(
      () =>
        fixture.container.textContent?.includes("Project code · Cowboy") ===
          true,
      "A project-only association should resolve its registered checkout",
    );
    check(
      !fixture.requests.some((request) =>
        request.includes("/sessions/fallback/")
      ),
      "Missing session cwd should use the registered project context",
    );
    check(
      !fixture.commands.some((command) =>
        ["prompt", "new_session", "resume_session", "set_config_option"]
          .includes(command)
      ),
      "Review activated or reconfigured an Agent worker",
    );
    tests.push(
      "project-only contexts fall back by workspace identity without starting an Agent worker",
    );
    return tests;
  } finally {
    fixture.cleanup();
  }
}
