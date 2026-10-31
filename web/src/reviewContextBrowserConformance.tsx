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
import { COARSE_POINTER_ROOT_CLASS } from "./platform";
import { Suspense, use } from "react";
import { MobileReviewBoundary } from "./mobile/review/MobileReviewPage";
import { MOBILE_OPEN_PRODUCT_EVENT, mobileProductFromEvent } from "./mobile/appPagerMotion";
import { getDraft, setDraft } from "./draftStore";

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

export function renderReviewContextPreview(dense = false, deferDefaults = false, branches = false, linked = false) {
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
    ...(linked
      ? [{
        id: "copy",
        cwd: "/runtime/copy",
        workspace_id: "cowboy",
        workspace_name: "Cowboy",
        workspace_source_path: "/projects/cowboy",
        machine_id: "ovh",
        execution_binding: {
          schema: 1,
          runtime: { machine_id: "ovh", cwd: "/runtime/copy" },
          environment: { machine_id: "hawk", protocol: 1 },
          workspace: { cwd: "/tasks/cowboy/copy" },
        },
        title: "Improve review (copy)",
        provider: "codex",
        status: "exited",
        continued_from: "worktree",
      } as SessionMeta]
      : []),
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
  const mutations: string[] = [];
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
        for (const session of sessions) this.publish({ type: "sync_patch", state: `mobile-review:${session.id}`, version: 0, resync: true, confirmed: [], value: { mode: "git", tabs: deferDefaults && session.id === "worktree" ? [{path:"README.md",pinned:false}] : [], ...(deferDefaults && session.id === "worktree" ? {active:"README.md"} : {}), progress: {}, positions: {} } });
        const defaults = () => {
          this.publish({ type: "sync_patch", state: "folders", version: 0, resync: true, confirmed: [], value: deferDefaults ? { folders: [{id:"initial",name:"Initial",parent:null,project:null,position:0,defaults:{review_directory:"workspace::hawk::suger"}}],placement:{worktree:"initial"} } : {folders:[],placement:{}} });
          this.publish({ type: "bootstrap_complete" });
        };
        if (deferDefaults) setTimeout(defaults, 180);
        else defaults();
      }, 20);
    }
    publish(message: unknown) {
      this.onmessage?.({ data: JSON.stringify(message) });
    }
    send(data: string) {
      commands.push(JSON.parse(data).type);
      const message = JSON.parse(data);
      if (message.type === "sync") mutations.push(`${message.state}/${message.name}`);
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
      const clean = context === "project" || context === "copy";
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
          comparisons: branches
            ? [
              { reference: "refs/remotes/origin/main", label: "origin/main" },
              { reference: "refs/remotes/origin/topic", label: "origin/topic" },
            ]
            : [],
          ...(branches ? { defaultComparison: "refs/remotes/origin/main" } : {}),
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
    mutations,
    refresh: () => socket?.publish({ type: "sessions", sessions }),
    sync: (state: string, value: unknown, version: number) => socket?.publish({ type: "sync_patch", state, value, version, confirmed: [], resync: true }),
    select: (id: string) => flushSync(() => setActiveSessionId(id)),
    cleanup: () => {
      root.unmount();
      container.remove();
      globalThis.fetch = originalFetch;
      globalThis.WebSocket = originalSocket;
    },
  };
}

async function checkReviewLoading(): Promise<string> {
  const container = document.createElement("div");
  container.style.cssText = "height:740px;width:390px";
  document.body.append(container);
  const root = createRoot(container);
  let resolve!: () => void;
  const pending = { promise: new Promise<void>((done) => { resolve = done; }), resolve: () => resolve() };
  const navigation: string[] = [];
  const onNavigate = (event: Event) => navigation.push((event as CustomEvent).detail.product);
  globalThis.addEventListener(MOBILE_OPEN_PRODUCT_EVENT, onNavigate);
  function SlowReview() {
    use(pending.promise);
    return <div data-review-ready>Ready</div>;
  }
  try {
    flushSync(() => root.render(
      <BrowserProductTheme>
        <CssBaseline />
        <Suspense fallback={<div data-whole-app-loading />}>
          <input aria-label="Agent draft" defaultValue="Keep this draft" />
          <div style={{ height: 680 }}>
            <MobileReviewBoundary><SlowReview /></MobileReviewBoundary>
          </div>
        </Suspense>
      </BrowserProductTheme>,
    ));
    await wait();
    const input = container.querySelector("input")!;
    const loading = container.querySelector<HTMLElement>("[data-mobile-review-loading]");
    check(loading && loading.getBoundingClientRect().height > 600, "Review loading must paint a full-height surface");
    check(!container.querySelector("[data-whole-app-loading]"), "Review loading hid Agent and the pager");
    check(loading.querySelector('[role="status"]')?.textContent?.includes("Opening review"), "Slow Review has no visible loading status");
    const back = loading.querySelector("button")!;
    const rect = back.getBoundingClientRect();
    check(rect.height >= 44 && rect.top > loading.getBoundingClientRect().top + 500, "Loading Review lost its bottom navigation");
    back.click();
    check(navigation[0] === "agent", "A slow Review cannot return to Agent");
    input.value = "Draft typed while Review loads";
    pending.resolve();
    await until(() => !!container.querySelector("[data-review-ready]"), "Review did not replace its loading surface");
    check(!container.querySelector("[data-mobile-review-loading]"), "Loading surface remained over Review");
    check(container.querySelector("input") === input && input.value === "Draft typed while Review loads", "Loading Review replaced the Agent editor");
    return "slow Review modules keep a visible loading surface and bottom escape without hiding or replacing Agent";
  } finally {
    pending.resolve();
    globalThis.removeEventListener(MOBILE_OPEN_PRODUCT_EVENT, onNavigate);
    root.unmount();
    container.remove();
  }
}

async function checkLinkedSessionReview(
  fixture: ReturnType<typeof renderReviewContextPreview>,
): Promise<string> {
  fixture.select("copy");
  const open = () => fixture.container.querySelector<HTMLElement>("[data-review-linked-session]");
  await until(() => !!open(), "A clean linked session should offer its source");
  check(fixture.container.textContent?.includes("Improve review"), "The linked session is not named");
  const before = fixture.requests.length;
  flushSync(() => open()!.click());
  await until(
    () => fixture.requests.slice(before).some((request) => request.includes("/api/code/sessions/worktree/changes")),
    "Reviewing the linked session should read its worktree",
  );
  check(getActiveSessionId() === "copy", "Reviewing a linked session moved the Agent");
  return "a clean session reviews its linked session's worktree without moving the Agent";
}

async function checkReviewCompareAndSelection(
  fixture: ReturnType<typeof renderReviewContextPreview>,
): Promise<string> {
  const body = document.body;
  const opened: unknown[] = [];
  const onOpen = (event: Event): void => {
    opened.push(mobileProductFromEvent(event));
  };
  window.addEventListener(MOBILE_OPEN_PRODUCT_EVENT, onOpen);
  const press = async (element: Element | null | undefined, name: string) => {
    check(element instanceof HTMLElement, `Missing ${name}`);
    flushSync(() => element.click());
    await wait();
  };
  const row = (name: string) =>
    [...body.querySelectorAll<HTMLElement>('[role="button"], button')]
      .find((element) => element.textContent?.includes(name));
  try {
    setDraft("worktree", { text: "", attachments: [] });
    fixture.refresh();
    const compare = () => fixture.container.querySelector("[data-review-compare]");
    await until(
      () => compare()?.textContent?.includes("Ready to merge · origin/main") === true,
      "The default comparison should be named by its scenario",
    );
    await until(
      () => fixture.container.querySelector("[data-review-unread]")?.textContent === "1 unread",
      "Unread count is missing",
    );

    await press(compare(), "comparison control");
    await until(() => !!row("Other branches") || !!row("origin/topic"), "Comparison picker did not open");
    await press(row("origin/topic"), "branch row");
    await until(
      () => compare()?.textContent?.includes("Branch · origin/topic") === true,
      "A chosen branch should become the comparison",
    );
    await until(
      () => fixture.mutations.some((mutation) => mutation.includes("setComparison")),
      "The chosen comparison was not saved to the session",
    );

    await until(() => !!row("app.ts"), "Comparison should list its files");
    await press(row("app.ts"), "changed file");
    const lines = () => [
      ...body.querySelectorAll<HTMLElement>(".cm-lineNumbers .cm-gutterElement"),
    ].filter((element) => element.textContent?.trim());
    const bar = () => body.querySelector("[data-review-selection-bar]");
    await until(() => lines().length >= 6, "Diff should render line numbers");
    await wait(300);
    await press(lines().find((line) => line.textContent?.trim() === "5"), "line 5");
    await until(() => bar()?.textContent?.includes("1 line") === true, "A tap should select its line");
    await press(lines().find((line) => line.textContent?.trim() === "6"), "line 6");
    await until(() => bar()?.textContent?.includes("2 lines") === true, "Two taps should select a range");
    check(body.querySelectorAll(".cm-review-selected-line").length === 2, "Selected lines are not highlighted");

    await press(row("Send to Agent"), "Send to Agent");
    await until(() => opened.includes("agent"), "Sending should return to the Agent");
    const draft = getDraft("worktree").text;
    check(
      draft.startsWith("`src/app.ts:1` (compared with origin/topic)\n```diff\n-old\n+new\n```"),
      `Draft should cite file lines, got: ${draft}`,
    );
    check(!bar(), "Sending should clear the selection");
    return "scenario comparison is saved to the session, and tapped lines reach the Agent draft as file lines";
  } finally {
    window.removeEventListener(MOBILE_OPEN_PRODUCT_EVENT, onOpen);
    setDraft("worktree", { text: "", attachments: [] });
  }
}

export async function runReviewContextBrowserConformance(): Promise<string[]> {
  const loadingResult = await checkReviewLoading();
  const fixture = renderReviewContextPreview(true, true);
  const tests: string[] = [loadingResult];
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
    await wait(90);
    check(!fixture.requests.some(r => r.includes("/api/code/")), "A hydrated tab fetched files before directory defaults loaded");
    await until(() => !!fixture.container.textContent?.includes("Project code · Suger"), "Late folder resync should select the inherited root");
    check(!fixture.requests.some(r => r.includes("/sessions/worktree/")), "An early Automatic root was pinned before folders loaded");
    fixture.sync("folders", { folders: [], placement: {} }, 1);
    fixture.sync("mobile-review:worktree", {mode:"git",tabs:[],progress:{},positions:{}}, 1);
    await until(() => !!fixture.container.textContent?.includes("Worktree · Cowboy"), "Reset inherited fixture");
    await wait(150);
    fixture.requests.length = 0;
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

    const modeButton = document.querySelector<HTMLElement>("[data-mobile-review-mode-switcher] [aria-pressed='true']")!;
    const restingFill = getComputedStyle(modeButton).backgroundColor;
    const hadCoarse = document.documentElement.classList.contains(COARSE_POINTER_ROOT_CLASS);
    document.documentElement.classList.add(COARSE_POINTER_ROOT_CLASS);
    modeButton.classList.add("Mui-focusVisible");
    check(restingFill !== "rgba(0, 0, 0, 0)" && getComputedStyle(modeButton).backgroundColor === restingFill,
      "touch focus must preserve the selected Review mode fill");
    modeButton.classList.remove("Mui-focusVisible");
    if (!hadCoarse) document.documentElement.classList.remove(COARSE_POINTER_ROOT_CLASS);
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
    const folders = (directory: string) => ({
      folders: [
        { id: "review-root", name: "Review root", project: null, parent: null, position: 0, defaults: { review_directory: directory } },
        { id: "review-child", name: "Review child", project: null, parent: "review-root", position: 0, defaults: {} },
      ], placement: { worktree: "review-child" },
    });
    fixture.sync("folders", folders("workspace::hawk::suger"), 2);
    fixture.select("worktree");
    await until(() => !!fixture.container.textContent?.includes("Project code · Suger"), "Directory default should choose the inherited project");
    await click(labelled("Worktree files"));
    await until(() => !!files()?.textContent?.includes("README.md"), "Inherited project should be browsable");
    await click(document.querySelector('[data-code-tree-path="README.md"]'));
    await until(() => !!document.querySelector("[data-markdown-review-preview]"), "Inherited project file should open");
    const countBeforeParentChange = fixture.requests.length;
    fixture.sync("folders", folders("workspace::hawk::cowboy"), 3);
    await wait(200);
    check(fixture.container.textContent?.includes("Project code · Suger"), "Parent default stole an open file's directory");
    check(!fixture.requests.slice(countBeforeParentChange).some(r => r.includes("workspace::hawk::cowboy")), "Parent default fetched the open file against another root");
    await click(labelled("Open context. Current project code Suger"));
    await until(() => !!document.querySelector("[data-session-review-directory]")?.textContent?.includes("Inherited from Review root"), "Default provenance is missing");
    await click(document.querySelector("[data-use-review-default]"));
    await until(() => !!fixture.container.textContent?.includes("Project code · Cowboy"), "Open default must apply the newest inherited directory");
    await click(labelled("Open context. Current project code Cowboy"));
    await click(document.querySelector("[data-edit-review-directory]"));
    await until(() => !!document.querySelector("[data-review-directory-picker]"), "Session default editor should use the shared picker");
    check(document.querySelector('[aria-label="Hawk code directories"]'), "Picker lost machine hierarchy");
    await click(document.querySelector('[data-review-directory="automatic"]'));
    await click(document.querySelector("[data-use-review-default]"));
    await until(() => !!fixture.container.textContent?.includes("Worktree · Cowboy"), "Automatic should block directory inheritance and use the real session worktree");
    check(getActiveSessionId() === "worktree", "Browsing defaults switched the Agent session");
    await click(labelled("Worktree files"));
    await until(() => !!files()?.textContent?.includes("README.md"), "Automatic worktree files");
    await click(document.querySelector('[data-code-tree-path="README.md"]'));
    await until(() => !!document.querySelector("[data-markdown-review-preview]"), "Automatic worktree document");
    await click(labelled("Close README.md"));
    await click([...document.querySelectorAll("button")].find(b => b.textContent?.trim() === "Close tab") ?? null);
    await click(labelled("Switch session. Current session Improve review"));
    await click(document.querySelector("[data-edit-review-directory]"));
    check(document.querySelector('[data-review-directory="automatic"]')?.classList.contains("Mui-selected"), "Closing the final tab erased the session directory override");
    await click(document.querySelector('[data-review-directory="session::project"]'));
    await click(document.querySelector("[data-use-review-default]"));
    await click(labelled("Worktree files"));
    await until(() => !!files()?.textContent?.includes("README.md"), "Another session worktree should be browsable");
    const beforeForeign = fixture.mutations.length;
    await click(document.querySelector('[data-code-tree-path="README.md"]'));
    await until(() => !!document.querySelector("[data-markdown-review-preview]"), "Foreign context file should open");
    await wait(600);
    check(!fixture.mutations.slice(beforeForeign).some(m => m.startsWith("mobile-review:project/")), "Browsing another session mutated its tabs or positions");
    check(!fixture.commands.some(c => ["prompt", "new_session", "resume_session", "set_config_option"].includes(c)), "Directory defaults activated a worker");
    tests.push("session and ancestor defaults share a hierarchical picker, preserve open files and return explicitly to the new default without worker activation");
    return tests;
  } finally {
    fixture.cleanup();
  }
}

export async function runReviewCompareBrowserConformance(): Promise<string[]> {
  const fixture = renderReviewContextPreview(false, false, true, true);
  try {
    return [
      await checkReviewCompareAndSelection(fixture),
      await checkLinkedSessionReview(fixture),
    ];
  } finally {
    fixture.cleanup();
  }
}
