/** Composer surfaces in the real App: the schedule dialog, the delivery
 * picker's Stop confirmation, the mobile branch switcher and the context
 * confirmation open and close through the shared slot
 * (composerSurfaces.ts). No account, server or agent: fetch and the WebSocket
 * are in-page fakes, as in desktopPerfBrowserFixture.tsx. */
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { App } from "./App";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { managementEntryFixture } from "./providerManagement.fixture";
import { loadProviderCatalog, resetProviderCatalog } from "./providerCatalogRegistry";
import { isMac } from "./platform";
import type { SessionMeta } from "./protocol";
import { composePendingPrompt } from "./contextPendingPrompt";
import { draftFromBlocks } from "./refusedMessage";
import { MessageWithActions } from "./MessageActions";
import { MessageBubble } from "./TranscriptItems";
import { closeMobileMessageActions } from "./messageCopyRegistry";

const SESSION = "surface-session";
/** The newest in-page socket, so a test can deliver a live transcript event. */
let liveSocket: { publish(message: unknown): void } | null = null;
const OTHER = "surface-destination";
const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
async function until<T>(read: () => T | null | undefined | false, what: string): Promise<T> {
  for (let i = 0; i < 200; i++) {
    const value = read();
    if (value) return value;
    await delay(25);
  }
  const panes = [...document.querySelectorAll("[data-desktop-pane]")].map((node) => node.getAttribute("data-desktop-pane"));
  const editors = [...document.querySelectorAll("textarea, [contenteditable=true]")].map((node) => node.className || node.tagName);
  throw new Error(`timed out waiting for ${what}; panes=${panes.join(",")} editors=${editors.join("|")} text=${document.body.innerText.replace(/\s+/g, " ").slice(0, 600)}`);
}
async function gone(read: () => unknown, what: string): Promise<void> {
  await until(() => !read(), `${what} to close`);
}
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
const byText = (selector: string, text: string) =>
  [...document.querySelectorAll<HTMLElement>(selector)].find((node) => node.textContent?.trim() === text);
const dialogTitled = (text: string) =>
  [...document.querySelectorAll<HTMLElement>("[role=dialog]")].find((node) => node.textContent?.includes(text));

/** The fixture sessions as the Controller broadcasts them; a test
 *  republishes them with another context reading. */
let fixtureSessions: SessionMeta[] = [];

function installFakes(sent: { type: string; session_id?: string }[]): void {
  const now = Date.now();
  const session = {
    id: SESSION,
    title: "Surface session",
    provider: "example",
    cwd: "/fixture/project",
    machine_id: "hawk",
    status: "busy",
    origin: "web",
    workspace_id: "project",
    workspace_name: "project",
    created_at_ms: now - 60_000,
    updated_at_ms: now,
    context_used: 40_000,
    context_size: 200_000,
  } as SessionMeta;
  // A second session gives the move-draft picker a destination and provides a child fork for the branch switcher.
  const other = { ...session, id: OTHER, title: "Destination session", status: "exited", forked_from: SESSION, updated_at_ms: now - 30_000 } as SessionMeta;
  fixtureSessions = [session, other];
  let queueVersion = 0;
  class FixtureSocket {
    static OPEN = 1;
    static CONNECTING = 0;
    static CLOSED = 3;
    readyState = 1;
    protocol = "cowboy-sync-v1";
    bufferedAmount = 0;
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: string }) => void) | null = null;
    onclose = null;
    onerror = null;
    constructor() {
      liveSocket = { publish: (message) => this.publish(message) };
      setTimeout(() => {
        this.onopen?.();
        this.publish({ type: "sessions", sessions: [session, other] });
        this.publish({ type: "bootstrap_complete" });
      }, 20);
    }
    publish(message: unknown) {
      this.onmessage?.({ data: JSON.stringify(message) });
    }
    send(data: string) {
      const frame = JSON.parse(data) as { type: string; session_id?: string; cmid?: string; text?: string; content?: unknown };
      sent.push(frame);
      // Acknowledge a saved draft the way the Controller does, so the row
      // leaves "Saving…" and offers its actions.
      if (frame.type === "add_draft" && frame.cmid) {
        setTimeout(() =>
          this.publish({
            type: "sync_patch",
            state: `queue:${frame.session_id}`,
            version: ++queueVersion,
            value: { queue: [], drafts: [{ id: frame.cmid, text: frame.text, content: frame.content, cmid: frame.cmid }] },
            confirmed: [frame.cmid],
          }), 10);
      }
    }
    close() {
      this.readyState = 3;
    }
  }
  globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;
  bindProductSyncPrincipal("surface-fixture");
  localStorage.setItem("cowboy:active", SESSION);
  const entry = managementEntryFixture("example");
  entry.publisher = entry.manifest.publisher;
  entry.manifest.authentication.required = false;
  entry.manifest.authentication.methods = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input instanceof Request ? input.url : input), location.href);
    if (!url.pathname.startsWith("/api/")) return originalFetch(input, init);
    if (url.pathname === "/api/sync/dataset") {
      return Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"b".repeat(64)}`,
        user_id: "surface-fixture",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      });
    }
    if (url.pathname === "/api/plugins") {
      return Response.json({ providers: [entry], authentications: [], authentication_executors: [] });
    }
    if (/^\/api\/sessions\/[^/]+\/bootstrap$/.test(url.pathname)) {
      return Response.json({
        messages: [{ type: "snapshot", session_id: SESSION, events: [], reached_start: true }],
      });
    }
    if (/^\/api\/sessions\/[^/]+\/lineage$/.test(url.pathname)) {
      return Response.json({
        session_id: SESSION,
        hops: [],
        branches: [
          {
            session_id: OTHER,
            title: "Destination session",
            provider: "example",
            kind: "native_fork",
            relation: "child",
            trashed: false,
          },
        ],
      });
    }
    if (url.pathname === "/api/usage") return Response.json({ providers: [], observed_at_ms: Date.now() });
    return new Response("Isolated fixture", { status: 404 });
  }) as typeof fetch;
}

function mount(surface: "desktop" | "touch", width: string): { root: Root; container: HTMLElement } {
  const container = document.createElement("div");
  container.style.cssText = `height:100vh;width:${width};overflow:hidden`;
  document.body.append(container);
  const root = createRoot(container);
  flushSync(() =>
    root.render(
      <BrowserProductTheme>
        <CssBaseline />
        <SurfaceProvider>
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <App themeMode="light" onSetThemeMode={() => {}} surface={surface} />
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </SurfaceProvider>
      </BrowserProductTheme>,
    )
  );
  return { root, container };
}

const editorIn = (container: HTMLElement) =>
  container.querySelector<HTMLElement>("[contenteditable=true], textarea");
const draftText = (container: HTMLElement): string => {
  const editor = editorIn(container);
  return editor instanceof HTMLTextAreaElement ? editor.value : editor?.textContent ?? "";
};
async function typeDraft(container: HTMLElement, text: string): Promise<void> {
  const editor = await until(() => editorIn(container), "composer editor");
  editor.focus();
  document.execCommand("selectAll");
  document.execCommand("insertText", false, text);
  await until(() => draftText(container).includes(text), "typed draft");
}

// A pending question like a Codex `request_user_input` form: long option
// descriptions and a free-text "Other" answer, beside a plan, so the bottom
// stack is as tall as it gets on a phone.
const QUESTION_OPTIONS = [
  ["Stop the Machine plan (recommended)", "Record the broker tombstone review conclusion and the reason it was abandoned in the docs, matching the widgets decision. Fix only the small traps this review exposed that already exist in production and need only a Controller fix, such as Stopping and Cancelled being written in two steps."],
  ["Keep building the full protocol", "Fill in the widgets section 8.3 ideas: reader floor, receiver checklist, per-command worker logs, terminal state persisted before the ACK, per-identity dedupe. Expect several review rounds and another Machine maintenance window."],
  ["Stop here", "No code changes, only write the review conclusions into the docs."],
] as const;

function questionEvents(): unknown[] {
  const properties: Record<string, unknown> = {
    next: {
      type: "string",
      title: "Next step",
      oneOf: QUESTION_OPTIONS.map(([title, description]) => ({ const: title, title, description })),
    },
    other: {
      type: "string",
      title: "Other",
      description: "Type your own answer, or add a note to the option you chose above (optional).",
    },
  };
  return [
    {
      session_id: SESSION,
      seq: 1,
      kind: "update",
      update: {
        sessionUpdate: "plan",
        entries: [{ content: "Managed delivery broker tombstone", priority: "medium", status: "pending" }],
      },
    },
    {
      session_id: SESSION,
      seq: 2,
      kind: "permission_request",
      request_id: "question-1",
      tool_call: {
        toolCallId: "question-tool-1",
        title: "Which direction next?",
        _cowboy: {
          elicitation: {
            mode: "form",
            sessionId: SESSION,
            toolCallId: "question-tool-1",
            message: "Which direction should the remaining work take?",
            requestedSchema: { type: "object", properties, required: ["next"] },
          },
        },
      },
      options: [
        { optionId: "accept", name: "Submit", kind: "allow_once" },
        { optionId: "decline", name: "Decline", kind: "reject_once" },
      ],
    },
  ];
}

/** Pretend the software keyboard covers part of the page, the way iOS
 *  reports it through visualViewport, so the app runs its keyboard layout. */
function emulateSoftwareKeyboard(initialCoverPx: number): { cover: (px: number) => void; restore: () => void } {
  const fake = Object.assign(new EventTarget(), {
    height: globalThis.innerHeight - initialCoverPx,
    width: globalThis.innerWidth,
    offsetTop: 0,
    offsetLeft: 0,
    pageTop: 0,
    pageLeft: 0,
    scale: 1,
  });
  const original = Object.getOwnPropertyDescriptor(globalThis, "visualViewport");
  Object.defineProperty(globalThis, "visualViewport", { configurable: true, value: fake });
  // useKeyboardInset publishes the overlap in the product entry; the column
  // pads itself by it, which is what shrinks the visible area.
  const root = document.documentElement.style;
  // The status strip floats over the top edge; the product publishes its height.
  root.setProperty("--cowboy-mobile-status-material-height", "59px");
  const cover = (px: number): void => {
    fake.height = globalThis.innerHeight - px;
    root.setProperty("--kb-inset", `${String(px)}px`);
    fake.dispatchEvent(new Event("resize"));
  };
  cover(initialCoverPx);
  return {
    cover,
    restore: () => {
      root.removeProperty("--kb-inset");
      root.removeProperty("--cowboy-mobile-status-material-height");
      if (original) Object.defineProperty(globalThis, "visualViewport", original);
      else delete (globalThis as { visualViewport?: unknown }).visualViewport;
    },
  };
}

/** The Mobile app with a long question and a plan pending, the keyboard up. */
async function mountQuestionScenario(coverPx: number): Promise<{
  container: HTMLElement;
  card: HTMLElement;
  keyboard: ReturnType<typeof emulateSoftwareKeyboard>;
  teardown: () => void;
}> {
  const keyboard = emulateSoftwareKeyboard(coverPx);
  const mobile = mount("touch", "390px");
  await until(() => editorIn(mobile.container) && liveSocket, "the Mobile composer");
  await delay(100);
  for (const envelope of questionEvents()) liveSocket?.publish({ type: "event", envelope });
  const card = await until(
    () => mobile.container.querySelector<HTMLElement>("[data-permission-overlay]"),
    "the pending question card",
  );
  return {
    container: mobile.container,
    card,
    keyboard,
    teardown: () => {
      mobile.root.unmount();
      mobile.container.remove();
      keyboard.restore();
    },
  };
}

const box = (node: Element | null | undefined) => {
  check(node, "a measured element is missing");
  const rect = node.getBoundingClientRect();
  return { top: Math.round(rect.top), bottom: Math.round(rect.bottom), shown: getComputedStyle(node).display !== "none" };
};

/** Where the answer card and its neighbours sit, in page pixels. */
function questionGeometry(container: HTMLElement) {
  const slot = (name: string) => container.querySelector(`[data-composer-stack-slot="${name}"]`);
  const card = container.querySelector("[data-permission-overlay] > div");
  const submit = [...container.querySelectorAll("[data-permission-overlay] button")]
    .find((button) => button.textContent?.trim() === "Submit");
  const heading = [...container.querySelectorAll("[data-permission-overlay] *")]
    .find((node) => node.children.length === 0 && node.textContent === "Answer needed");
  const kbInset = Number.parseFloat(document.documentElement.style.getPropertyValue("--kb-inset") || "0");
  return {
    visibleBottom: globalThis.innerHeight - kbInset,
    card: box(card),
    heading: box(heading),
    submit: box(submit),
    plan: box(slot("plan")),
    primary: box(slot("primary")),
  };
}

/** Leave the keyboard-up Mobile question on screen for a visual review. */
export async function renderQuestionKeyboardPreview(coverPx = 268): Promise<unknown> {
  installFakes([]);
  resetProviderCatalog();
  await loadProviderCatalog();
  const scenario = await mountQuestionScenario(coverPx);
  const other = await until(
    () => scenario.card.querySelector<HTMLElement>("textarea:not([aria-hidden])"),
    "the Other answer field",
  );
  other.focus();
  await delay(600);
  return questionGeometry(scenario.container);
}

/** A busy session whose newest row is a running tool call, on a phone layout. */
async function mountRunningToolScenario(): Promise<{ container: HTMLElement; teardown: () => void }> {
  const mobile = mount("touch", "390px");
  await until(() => editorIn(mobile.container) && liveSocket, "the Mobile composer");
  await delay(100);
  const event = (seq: number, update: Record<string, unknown>) =>
    liveSocket?.publish({ type: "event", envelope: { session_id: SESSION, seq, kind: "update", update } });
  event(1, { sessionUpdate: "user_message_chunk", content: { type: "text", text: "run the checks" } });
  for (let i = 0; i < 10; i++) {
    event(2 + i, {
      sessionUpdate: "tool_call",
      toolCallId: `done-${i}`,
      title: `cd /srv/storage/fast0/agent/worktrees/cowboy/example && step ${i}`,
      kind: "execute",
      status: "completed",
    });
  }
  event(20, {
    sessionUpdate: "tool_call",
    toolCallId: "running",
    title: "timeout 900 nix develop -c bash -c 'for s in elicitation subagents'",
    kind: "execute",
    status: "pending",
  });
  await until(() => mobile.container.querySelector("[data-transcript-tail-row=activity]"), "the activity row under the running card");
  await delay(300);
  return {
    container: mobile.container,
    teardown: () => {
      mobile.root.unmount();
      mobile.container.remove();
    },
  };
}

/** The tool row's card and the row under it, measured in page pixels. */
function runningToolGeometry(container: HTMLElement) {
  const row = container.querySelector<HTMLElement>("[data-key]");
  const card = row?.firstElementChild as HTMLElement | null | undefined;
  check(row && card, "the running tool row is missing");
  const activity = container.querySelector("[data-transcript-tail-row=activity]");
  check(activity, "the activity row is missing");
  const animation = card.getAnimations()[0];
  return {
    cardBottom: Math.round(card.getBoundingClientRect().bottom),
    activityTop: Math.round(activity.getBoundingClientRect().top),
    animated: animation
      ? (animation.effect as KeyframeEffect).getKeyframes().flatMap((frame) => Object.keys(frame))
      : null,
    opacity: getComputedStyle(card).opacity,
  };
}

/** Leave the running-tool tail on screen for a visual review. */
export async function renderTranscriptTailPreview(): Promise<unknown> {
  installFakes([]);
  resetProviderCatalog();
  await loadProviderCatalog();
  const scenario = await mountRunningToolScenario();
  return runningToolGeometry(scenario.container);
}

/** Trusted-key flow (tools/cdp-keyboard-acceptance.ts `surfaces`): the same
 *  fakes and Desktop App, with sent frames exposed for the runner. */
export async function mountComposerSurfacesKeyboardAcceptance(): Promise<void> {
  const sent: { type: string; session_id?: string }[] = [];
  (globalThis as { __surfaceSent?: typeof sent }).__surfaceSent = sent;
  installFakes(sent);
  resetProviderCatalog();
  await loadProviderCatalog();
  const desktop = mount("desktop", "100vw");
  (await until(
    () => [...desktop.container.querySelectorAll<HTMLElement>("[data-desktop-session-row]")].find((row) => row.textContent?.includes("Surface session")),
    "session row",
  )).click();
  await until(() => editorIn(desktop.container), "composer editor");
}

const reading = (used: number, compactedAtMs?: number): Partial<SessionMeta> => ({
  context_used: used,
  context_size: 200_000,
  usage: {
    used,
    size: 200_000,
    raw: null,
    observed_at_ms: Date.now(),
    ...(compactedAtMs === undefined ? {} : { compacted_at_ms: compactedAtMs }),
  },
});

/** Republish the fixture sessions, the active one idle with this reading. */
function publishReading(used: number, compactedAtMs?: number): void {
  check(liveSocket, "the fixture socket");
  liveSocket.publish({
    type: "sessions",
    sessions: fixtureSessions.map((session) =>
      session.id === SESSION ? { ...session, status: "running", ...reading(used, compactedAtMs) } : session
    ),
  });
}

const desktopMeter = (container: HTMLElement) =>
  container.querySelector<HTMLElement>("[data-composer-action=compact]");
const mobileMeter = (container: HTMLElement) =>
  container.querySelector<HTMLElement>("[aria-label='compact conversation']");

/** The Composer's context meter before (`stale` false) or after a compaction
 *  that left the reading as history, for a screenshot of either surface. */
export async function mountCompactionMeterPreview(
  surface: "desktop" | "touch",
  stale: boolean,
): Promise<void> {
  installFakes([]);
  resetProviderCatalog();
  await loadProviderCatalog();
  const { container } = mount(surface, surface === "desktop" ? "1280px" : "390px");
  const meter = surface === "desktop" ? desktopMeter : mobileMeter;
  await until(() => meter(container) && liveSocket, "the context meter");
  publishReading(94_000, stale ? Date.now() : undefined);
  await until(
    () => meter(container)?.textContent?.includes(stale ? "—" : "47"),
    "the published reading",
  );
}

/** A compaction that no usage report followed shows the meter as unknown on
 *  both surfaces; the next report shows the compacted context. */
async function compactionMeter(surface: "desktop" | "touch"): Promise<string> {
  const { root, container } = mount(surface, surface === "desktop" ? "1280px" : "390px");
  const meter = () => (surface === "desktop" ? desktopMeter : mobileMeter)(container);
  const hint = () =>
    surface === "desktop"
      ? meter()?.getAttribute("aria-label") ?? ""
      : meter()?.closest("[aria-label^='Compact conversation']")?.getAttribute("aria-label") ?? "";
  await until(() => meter() && liveSocket, `the ${surface} context meter`);
  publishReading(94_000);
  await until(() => meter()?.textContent?.includes("47"), `the ${surface} 47% reading`);
  publishReading(94_000, Date.now());
  await until(() => meter()?.textContent?.includes("—"), `the ${surface} unknown reading`);
  check(!meter()?.textContent?.includes("47"), `the ${surface} meter drops the old percentage`);
  check(hint().includes("Compacted · updates after the next reply"), `the ${surface} hint (${hint()})`);
  publishReading(24_000);
  await until(() => meter()?.textContent?.includes("12"), `the ${surface} fresh reading`);
  check(!meter()?.textContent?.includes("—"), `the ${surface} meter is known again`);
  liveSocket?.publish({ type: "sessions", sessions: fixtureSessions });
  root.unmount();
  container.remove();
  return `${surface === "desktop" ? "Desktop" : "Mobile"} context meter shows unknown after a compaction until the next usage report`;
}

export async function runComposerSurfacesBrowserConformance(): Promise<string[]> {
  const sent: { type: string; session_id?: string }[] = [];
  installFakes(sent);
  resetProviderCatalog();
  await loadProviderCatalog();
  const passed: string[] = [];

  // Desktop: the toolbar's schedule action and the delivery picker's Stop.
  const desktop = mount("desktop", "1280px");
  (await until(
    () => [...desktop.container.querySelectorAll<HTMLElement>("[data-desktop-session-row]")].find((row) => row.textContent?.includes("Surface session")),
    "session row",
  )).click();
  await typeDraft(desktop.container, "surface check");
  (await until(() => desktop.container.querySelector<HTMLElement>("[data-composer-action=schedule]:not([disabled])"), "schedule action")).click();
  await until(() => document.querySelector("[aria-label='Send time']"), "schedule dialog");
  // A synthetic Escape is not trusted input for the Desktop key dispatcher;
  // the backdrop reaches the same onClose.
  const backdrop = await until(() => [...document.querySelectorAll<HTMLElement>(".MuiBackdrop-root")].at(-1), "schedule backdrop");
  backdrop.click();
  await gone(() => document.querySelector("[aria-label='Send time']"), "schedule dialog");
  passed.push("Desktop schedule dialog opens from the toolbar and dismissing it closes it");

  (await until(() => desktop.container.querySelector<HTMLElement>("[data-composer-action=send]:not([disabled])"), "send action")).click();
  const picker = await until(() => document.querySelector<HTMLElement>("[aria-label='Choose delivery']"), "delivery picker");
  picker.dispatchEvent(new KeyboardEvent("keydown", {
    key: ".",
    code: "Period",
    metaKey: isMac,
    ctrlKey: !isMac,
    bubbles: true,
    cancelable: true,
  }));
  const stop = await until(() => dialogTitled("Stop the running turn?"), "stop confirmation");
  await gone(() => document.querySelector("[aria-label='Choose delivery']"), "delivery picker");
  const confirm = [...stop.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.trim() === "Stop");
  check(confirm, "stop confirmation button");
  confirm.click();
  await gone(() => dialogTitled("Stop the running turn?"), "stop confirmation");
  check(sent.some((frame) => frame.type === "cancel" && frame.session_id === SESSION), "confirming Stop cancels the turn");
  passed.push("Delivery picker Stop replaces the picker with the stop confirmation, which cancels the turn");
  desktop.root.unmount();
  desktop.container.remove();

  passed.push(await compactionMeter("desktop"));
  passed.push(await compactionMeter("touch"));

  // Mobile: clear-composer button is gone, and the fork branch switcher sits at the end with GitForkIcon.
  const mobile = mount("touch", "390px");
  const actionTrack = await until(
    () => mobile.container.querySelector<HTMLElement>("[data-mobile-scrollable-actions='true']"),
    "mobile composer action track",
  );
  check(!mobile.container.querySelector("[aria-label='clear composer']"), "Clear composer button is removed");
  const branchButton = await until(
    () => actionTrack.querySelector<HTMLElement>("[data-mobile-branch-switcher]"),
    "mobile branch switcher button",
  );
  const scheduleButton = await until(
    () => actionTrack.querySelector<HTMLElement>("[aria-label='schedule send']"),
    "mobile schedule button",
  );
  const actionButtons = [...actionTrack.querySelectorAll<HTMLElement>("button")];
  check(actionButtons.at(-1) === branchButton, "Branch switcher is the last button in the mobile composer action row");
  check(
    branchButton.getBoundingClientRect().left >= scheduleButton.getBoundingClientRect().right - 1,
    "Branch switcher renders to the right of Schedule send",
  );
  check(branchButton.querySelector("[data-icon='git-fork']") !== null, "Branch switcher renders the GitFork icon");
  branchButton.click();
  const branchSheet = await until(() => dialogTitled("Branches & lineage"), "branches and lineage sheet");
  const closeBranches = await until(
    () => [...branchSheet.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "Close" || button.textContent?.trim() === "Close"),
    "branches sheet close button",
  );
  closeBranches.click();
  await gone(() => dialogTitled("Branches & lineage"), "branches and lineage sheet");
  passed.push("Mobile composer omits Clear composer and places the GitFork branch switcher last");

  (await until(() => mobile.container.querySelector<HTMLElement>("[aria-label='clear conversation']:not([disabled])"), "clear conversation button")).click();
  const context = await until(() => document.querySelector<HTMLElement>("[role=dialog]"), "context confirmation");
  const cancel = byText("[role=dialog] button", "Cancel");
  check(cancel, "context confirmation cancel");
  cancel.click();
  await gone(() => context.isConnected && document.querySelector("[role=dialog]"), "context confirmation");
  check(!sent.some((frame) => frame.type === "reset" || frame.type === "clear_context"), "cancelling sends no context action");
  passed.push("Mobile clear-conversation confirmation opens and Cancel closes it without a request");

  // Full-screen is a flag beside the slot: expanding carries the inline draft
  // in, collapsing carries the edited draft back to the inline editor.
  await typeDraft(mobile.container, "fullscreen draft");
  (await until(() => mobile.container.querySelector<HTMLElement>("[aria-label='fullscreen editor']"), "fullscreen button")).click();
  const sheet = () => document.querySelector<HTMLElement>("[aria-label='Fullscreen message editor']");
  const fullscreen = await until(sheet, "fullscreen editor");
  await until(() => draftText(fullscreen).includes("fullscreen draft"), "the draft carried into full-screen");
  const fullscreenEditor = editorIn(fullscreen)!;
  fullscreenEditor.focus();
  document.execCommand("insertText", false, " edited");
  (await until(() => fullscreen.querySelector<HTMLElement>("[data-testid=CloseFullscreenIcon]")?.closest<HTMLElement>("button"), "collapse button")).click();
  await gone(sheet, "fullscreen editor");
  await until(() => draftText(mobile.container).includes("edited"), "the edit carried back inline");
  passed.push("Full-screen editor: expanding carries the draft in, collapsing carries the edit back inline");

  const customize = await until(
    () => mobile.container.querySelector<HTMLElement>("[data-mobile-focus-format-row] [data-testid=TuneIcon]")?.closest<HTMLElement>("button"),
    "customize toolbar button",
  );
  customize.click();
  const toolbarSheet = () => document.querySelector<HTMLElement>("[aria-label='Toolbar options']");
  await until(toolbarSheet, "toolbar options sheet");
  const closeSheet = await until(() => toolbarSheet()?.querySelector<HTMLElement>("[aria-label=Close]"), "toolbar sheet close");
  closeSheet.click();
  await gone(toolbarSheet, "toolbar options sheet");
  passed.push("Mobile toolbar options open from the format row and Close dismisses them");

  await typeDraft(mobile.container, "draft to move");
  (await until(() => mobile.container.querySelector<HTMLElement>("[aria-label='save as draft']:not([disabled])"), "save as draft")).click();
  const moveAction = () => mobile.container.querySelector<HTMLElement>("[aria-label='Move to another session…']");
  (await until(moveAction, "draft move action")).click();
  const destination = () => byText("[role=dialog] *", "Destination session");
  await until(destination, "move destination picker");
  const pickerDialog = destination()!.closest<HTMLElement>("[role=dialog]")!;
  const dismiss = pickerDialog.querySelector<HTMLElement>("[aria-label=Close]") ??
    [...document.querySelectorAll<HTMLElement>(".MuiBackdrop-root")].at(-1);
  check(dismiss, "move picker dismiss control");
  dismiss.click();
  await gone(destination, "move destination picker");
  check(!sent.some((frame) => frame.type === "move_draft"), "dismissing the picker moves nothing");
  (await until(moveAction, "draft move action")).click();
  (await until(destination, "move destination picker")).click();
  await until(() => sent.some((frame) => frame.type === "move_draft" && frame.session_id === SESSION), "move_draft request");
  await gone(destination, "move destination picker");
  passed.push("Draft move picker: dismissing moves nothing, choosing a destination sends move_draft and closes it");
  mobile.root.unmount();
  mobile.container.remove();

  // A long question beside a plan, answered on the software keyboard: the
  // card owns the stack, stays whole inside the visible area, and the stack
  // returns when the keyboard leaves.
  const question = await mountQuestionScenario(300);
  const inside = (g: ReturnType<typeof questionGeometry>, label: string, minTop: number) => {
    check(g.card.top >= minTop, `${label}: the card starts below ${minTop}px (${g.card.top})`);
    check(g.card.bottom <= g.visibleBottom + 1, `${label}: the card ends above the keyboard (${g.card.bottom} > ${g.visibleBottom})`);
    check(g.heading.top >= g.card.top && g.heading.bottom <= g.card.bottom, `${label}: the heading is inside the card`);
    check(g.submit.top >= g.card.top && g.submit.bottom <= g.card.bottom, `${label}: Submit is inside the card`);
  };
  const other = await until(
    () => question.card.querySelector<HTMLElement>("textarea:not([aria-hidden])"),
    "the Other answer field",
  );
  other.focus();
  await until(() => question.card.getAttribute("data-answer-keyboard") === "true", "the answer keyboard layout");
  await delay(100);
  const typing = questionGeometry(question.container);
  inside(typing, "typing an answer", 59); // the status strip floats over the top edge
  check(!typing.plan.shown && !typing.primary.shown, "Plan and the Composer give way to the card while its answer is typed");
  question.keyboard.cover(0);
  other.blur();
  await until(() => question.card.getAttribute("data-answer-keyboard") !== "true", "the answer keyboard layout to end");
  await delay(100);
  const resting = questionGeometry(question.container);
  inside(resting, "keyboard closed", 0);
  check(resting.plan.shown && resting.primary.shown, "Plan and the Composer return when the keyboard leaves");
  question.teardown();
  passed.push("A long question stays whole above the keyboard and owns the stack only while its answer is typed");

  // A running tool card breathes by its fill only. An opacity animation makes
  // it an iOS compositor layer that was left painted one row low, over the
  // activity row beneath it, once that row changed height.
  const tail = await mountRunningToolScenario();
  const running = runningToolGeometry(tail.container);
  check(running.animated !== null && running.animated.includes("backgroundColor"), `the running card breathes (${String(running.animated)})`);
  check(!running.animated.some((property: string) => ["opacity", "transform", "filter", "backdropFilter"].includes(property)), `the breath stays off the compositor (${running.animated.join(",")})`);
  check(running.opacity === "1", `the card itself is never dimmed (${running.opacity})`);
  check(running.cardBottom <= running.activityTop, `the card ends above the activity row (${running.cardBottom} > ${running.activityTop})`);
  tail.teardown();
  passed.push("A running tool card breathes off the compositor and stays above the activity row");

  // Back to Prompt (refusedMessage.ts): a refused message returns to the real
  // Composer with its image as an inline thumbnail where it was sent.
  const putBack = mount("desktop", "1200px");
  await until(() => editorIn(putBack.container), "the Desktop composer");
  const { draft } = await draftFromBlocks([
    { type: "text", text: "put back " },
    {
      type: "image",
      mimeType: "image/png",
      data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    },
    { type: "text", text: " with its image" },
  ]);
  composePendingPrompt(SESSION, draft);
  await until(
    () => putBack.container.querySelector(".cm-content img.cm-inline-image"),
    "the put-back image thumbnail",
  );
  check(
    draftText(putBack.container).startsWith("put back ") &&
      draftText(putBack.container).includes(" with its image"),
    `the put-back text is in the Composer (${draftText(putBack.container)})`,
  );
  putBack.root.unmount();
  putBack.container.remove();
  passed.push("Back to Prompt returns a refused message to the real Composer with its image inline");

  // "New session here" from a folder's menu: the menu must be gone before the
  // opener claims the keyboard, or its focus restore blurs the claim inside
  // the same tap and Create opens with a caret but no keyboard.
  const filed = mount("touch", "390px");
  await until(() => editorIn(filed.container) && liveSocket, "the Mobile composer");
  liveSocket?.publish({
    type: "sync_patch",
    state: "folders",
    version: 1,
    value: { folders: [{ id: "f1", name: "Project", parent: null, project: null, position: 0 }], placement: { [SESSION]: "f1" } },
    confirmed: [],
    resync: true,
  });
  const kebab = await until(() => filed.container.querySelector<HTMLElement>("[aria-label='folder actions Project']"), "the folder actions button");
  kebab.focus();
  kebab.click();
  const item = await until(() => byText("[role=menuitem]", "New session here"), "the New session here item");
  item.click();
  const claim = document.activeElement;
  check(
    claim instanceof HTMLInputElement && claim.getAttribute("aria-hidden") === "true" && !claim.closest("[role=dialog]"),
    `the keyboard claim keeps focus through the tap (${String(claim?.tagName)} ${claim?.getAttribute("aria-label") ?? ""})`,
  );
  const titleField = await until(() => document.querySelector<HTMLInputElement>("[role=dialog] input[type=text], [role=dialog] input:not([type])"), "the Create title field");
  await until(() => document.activeElement === titleField, "focus to transfer to the Create title");
  filed.root.unmount();
  filed.container.remove();
  passed.push("New session here from a folder menu keeps the keyboard claim until Create's title takes focus");

  // Hidden Copy + Copy as Markdown (Desktop): zero-height hover pills at rest,
  // top-edge mirror on tall replies (>480px), dual-MIME Copy (HTML + clean plain),
  // verbatim Copy as Markdown, streaming exclusion, and Conversation Vim y / Y.
  {
    const writtenPlain: string[] = [];
    const writtenHtml: string[] = [];
    const writtenRaw: string[] = [];
    const origClipboard = navigator.clipboard;
    const origClipboardItem = globalThis.ClipboardItem;
    class StubClipboardItem {
      readonly map: Record<string, Blob>;
      constructor(map: Record<string, Blob>) { this.map = map; }
    }
    Object.defineProperty(globalThis, "ClipboardItem", { value: StubClipboardItem, configurable: true });
    Object.defineProperty(navigator, "clipboard", {
      value: {
        write: async (items: StubClipboardItem[]) => {
          for (const entry of items) {
            if (entry.map["text/plain"]) writtenPlain.push(await entry.map["text/plain"].text());
            if (entry.map["text/html"]) writtenHtml.push(await entry.map["text/html"].text());
          }
        },
        writeText: async (text: string) => { writtenRaw.push(text); },
      },
      configurable: true,
    });
    const tallMarkdown = [
      "### Architecture Summary",
      "",
      "- **Core**: inspect [App.tsx](file:///home/draven/App.tsx) and [Docs](https://example.com/docs)",
      "",
      "| Tool | Mechanism |",
      "| --- | --- |",
      "| **Cowboy** | `MessageWithActions` |",
      "",
      ...Array.from({ length: 24 }, (_, idx) => `Paragraph line ${String(idx + 1)} with \`snake_case_${String(idx)}\` detail.`),
    ].join("\n");
    const host = document.createElement("div");
    host.style.cssText = "position:fixed;inset:0;width:900px;height:720px;overflow:auto;z-index:2000;background:#fff;";
    document.body.appendChild(host);
    const copyRoot = createRoot(host);
    try {
      const tallItem = { key: "m-tall", kind: "message" as const, role: "assistant" as const, chunks: [{ type: "text" as const, text: tallMarkdown }] };
      const streamItem = { key: "m-stream", kind: "message" as const, role: "assistant" as const, chunks: [{ type: "text" as const, text: "Still generating..." }] };
      const userItem = { key: "m-user", kind: "message" as const, role: "user" as const, chunks: [{ type: "text" as const, text: "Check **this** prompt" }] };
      flushSync(() => {
        copyRoot.render(
          <BrowserProductTheme>
            <SurfaceProvider>
              <DesktopWorkspaceProvider>
                <DesktopCommandProvider>
                  <div
                    data-desktop-pane="conversation"
                    data-desktop-region="conversation.transcript"
                    data-desktop-navigation="scroll"
                    style={{ height: 680, overflow: "auto", padding: 24, display: "flex", flexDirection: "column", gap: 16 }}
                  >
                    <div data-desktop-transcript-scroller="" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
                      <MessageWithActions item={userItem} sessionId={SESSION} streaming={false} desktop>
                        <MessageBubble role="user" chunks={userItem.chunks} provider="codex" desktop />
                      </MessageWithActions>
                      <MessageWithActions item={tallItem} sessionId={SESSION} streaming={false} desktop>
                        <MessageBubble role="assistant" chunks={tallItem.chunks} provider="codex" desktop />
                      </MessageWithActions>
                      <MessageWithActions item={streamItem} sessionId={SESSION} streaming desktop>
                        <MessageBubble role="assistant" chunks={streamItem.chunks} streaming provider="codex" desktop />
                      </MessageWithActions>
                    </div>
                  </div>
                </DesktopCommandProvider>
              </DesktopWorkspaceProvider>
            </SurfaceProvider>
          </BrowserProductTheme>,
        );
      });
      // Streaming message never mounts a copy wrapper or action bar
      check(!host.querySelector('[data-message-copy-id$=":m-stream"]'), "streaming message omits copy wrapper");
      const tallWrap = await until(() => host.querySelector<HTMLElement>('[data-message-copy-id$=":m-tall"]'), "tall assistant copy wrapper");
      const userWrap = await until(() => host.querySelector<HTMLElement>('[data-message-copy-id$=":m-user"]'), "user copy wrapper");
      const bottomBar = tallWrap.querySelector<HTMLElement>('[data-message-action-bar="bottom"]')!;
      check(Boolean(bottomBar), "assistant bubble mounts bottom hover action bar");
      check(
        getComputedStyle(bottomBar).opacity === "0" && getComputedStyle(bottomBar).pointerEvents === "none",
        `hover bar stays visually hidden at rest without visibility:hidden (${getComputedStyle(bottomBar).opacity}/${getComputedStyle(bottomBar).pointerEvents})`,
      );
      // Primary bottom pill must be keyboard-focusable (not trapped by visibility:hidden)
      const focusableCopy = bottomBar.querySelector<HTMLButtonElement>('[data-message-copy="rich"]')!;
      focusableCopy.focus();
      check(document.activeElement === focusableCopy, "bottom pill Copy button receives keyboard focus directly");
      check(getComputedStyle(bottomBar).opacity === "1" || bottomBar.matches(":focus-within"), "focusing bottom pill reveals it via :focus-within");
      focusableCopy.blur();
      check(Boolean(userWrap.querySelector('[data-message-action-bar="bottom"]')), "user bubble mounts bottom hover action bar");
      const topBar = await until(() => tallWrap.querySelector<HTMLElement>('[data-message-action-bar="top"]'), "tall reply top-edge mirror pill");
      check(topBar.querySelector<HTMLButtonElement>('[data-message-copy="rich"]')?.tabIndex === -1, "top mirror buttons skip duplicate Tab stops");

      // Click Copy -> writes dual text/html + clean text/plain and flips to Copied
      const copyBtn = bottomBar.querySelector<HTMLButtonElement>('[data-message-copy="rich"]')!;
      copyBtn.click();
      await until(() => writtenPlain.length === 1 && writtenHtml.length === 1, "dual-MIME Copy write");
      check(writtenHtml[0]!.includes("<table") && writtenHtml[0]!.includes("<code>App.tsx</code>") && !writtenHtml[0]!.includes("file:///"), `clean HTML clipboard (${writtenHtml[0]!.slice(0, 120)})`);
      check(writtenPlain[0]!.includes("Cowboy\tMessageWithActions") && !writtenPlain[0]!.includes("###"), `clean plain text clipboard (${writtenPlain[0]!.slice(0, 120)})`);
      await until(() => copyBtn.textContent?.includes("Copied") === true, "Copied button confirmation");

      // Click Markdown -> writes verbatim raw Markdown
      const mdBtn = bottomBar.querySelector<HTMLButtonElement>('[data-message-copy="markdown"]')!;
      mdBtn.click();
      await until(() => writtenRaw.length === 1, "raw Markdown clipboard write");
      check(writtenRaw[0] === tallMarkdown, "Copy as Markdown preserves exact raw Markdown source");

      // Conversation Vim y / Y yanks focused/viewport message without needing bottom scroll
      const convRegion = host.querySelector<HTMLElement>('[data-desktop-region="conversation.transcript"]')!;
      convRegion.tabIndex = -1;
      convRegion.focus();
      await delay(50);
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "y", code: "KeyY", bubbles: true, cancelable: true }));
      await until(() => writtenPlain.length === 2, "Vim y yank write");
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Y", code: "KeyY", shiftKey: true, bubbles: true, cancelable: true }));
      await until(() => writtenRaw.length === 2, "Vim Y Markdown yank write");
      check(writtenRaw[1] === tallMarkdown, "Vim Y yanks verbatim Markdown of viewport-anchored reply");
    } finally {
      copyRoot.unmount();
      host.remove();
      Object.defineProperty(navigator, "clipboard", { value: origClipboard, configurable: true });
      Object.defineProperty(globalThis, "ClipboardItem", { value: origClipboardItem, configurable: true });
    }
    passed.push("Desktop hidden message Copy/Markdown pills stay zero-height, mirror at top for tall replies, and respond to Vim y/Y");
  }

  // Hidden Copy + Copy as Markdown (Mobile): zero chrome at rest, no Fork chrome,
  // long-press/drag/link taps stay native, stationary prose tap pops clamped floating pill.
  {
    const writtenPlain: string[] = [];
    const writtenRaw: string[] = [];
    const origClipboard = navigator.clipboard;
    const origClipboardItem = globalThis.ClipboardItem;
    class StubClipboardItem {
      readonly map: Record<string, Blob>;
      constructor(map: Record<string, Blob>) { this.map = map; }
    }
    Object.defineProperty(globalThis, "ClipboardItem", { value: StubClipboardItem, configurable: true });
    Object.defineProperty(navigator, "clipboard", {
      value: {
        write: async (items: StubClipboardItem[]) => {
          for (const entry of items) {
            if (entry.map["text/plain"]) writtenPlain.push(await entry.map["text/plain"].text());
          }
        },
        writeText: async (text: string) => { writtenRaw.push(text); },
      },
      configurable: true,
    });
    closeMobileMessageActions();
    const host = document.createElement("div");
    host.style.cssText = "position:fixed;left:0;top:0;width:390px;height:780px;overflow:auto;z-index:2000;background:#fff;padding:16px;";
    document.body.appendChild(host);
    const mobileRoot = createRoot(host);
    try {
      const mobileItem = {
        key: "m-mob",
        kind: "message" as const,
        role: "assistant" as const,
        chunks: [{ type: "text" as const, text: "Tap **prose** or open [Example](https://example.com)" }],
      };
      flushSync(() => {
        mobileRoot.render(
          <BrowserProductTheme>
            <SurfaceProvider>
              <MessageWithActions item={mobileItem} sessionId={SESSION} streaming={false} desktop={false}>
                <MessageBubble role="assistant" chunks={mobileItem.chunks} provider="codex" desktop={false} />
              </MessageWithActions>
            </SurfaceProvider>
          </BrowserProductTheme>,
        );
      });
      const wrap = await until(() => host.querySelector<HTMLElement>('[data-message-copy-id$=":m-mob"]'), "mobile message wrapper");
      check(!document.querySelector('[data-message-action-bar]'), "Mobile keeps zero action chrome at rest");
      check(!document.querySelector('[data-fork-from-here]'), "Mobile never renders inline Fork chrome");

      // Secondary mouse/trackpad click must NOT open Mobile copy pill
      wrap.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 9, pointerType: "mouse", button: 2, clientX: 160, clientY: 180 }));
      wrap.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, isPrimary: true, pointerId: 9, pointerType: "mouse", button: 2, clientX: 160, clientY: 180 }));
      await delay(30);
      check(!document.querySelector('[data-message-action-bar="mobile"]'), "secondary button click never opens Mobile copy pill");

      // Drag (>10px) must NOT open pill
      wrap.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 1, pointerType: "touch", clientX: 120, clientY: 160 }));
      wrap.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, isPrimary: true, pointerId: 1, pointerType: "touch", clientX: 120, clientY: 195 }));
      wrap.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, isPrimary: true, pointerId: 1, pointerType: "touch", clientX: 120, clientY: 195 }));
      await delay(40);
      check(!document.querySelector('[data-message-action-bar="mobile"]'), "scroll/drag gesture does not open Mobile copy pill");

      // Long-press (>450ms) must NOT hijack native iOS text selection
      wrap.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 2, pointerType: "touch", clientX: 120, clientY: 160 }));
      await delay(500);
      wrap.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, isPrimary: true, pointerId: 2, pointerType: "touch", clientX: 120, clientY: 160 }));
      await delay(40);
      check(!document.querySelector('[data-message-action-bar="mobile"]'), "long-press preserves native selection without popping copy pill");

      // Stationary tap on prose opens portaled viewport-clamped Mobile pill
      wrap.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 3, pointerType: "touch", clientX: 160, clientY: 180 }));
      wrap.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, isPrimary: true, pointerId: 3, pointerType: "touch", clientX: 160, clientY: 180 }));
      const mobileBar = await until(() => document.querySelector<HTMLElement>('[data-message-action-bar="mobile"]'), "Mobile floating copy pill");
      const barRect = mobileBar.getBoundingClientRect();
      check(barRect.left >= 12 && barRect.right <= window.innerWidth - 12 && barRect.top >= 56, `Mobile floating pill stays clamped inside viewport (${JSON.stringify(barRect)})`);

      // Tap Markdown -> writes exact raw Markdown and auto-dismisses
      const mdBtn = mobileBar.querySelector<HTMLButtonElement>('[data-message-copy="markdown"]')!;
      mdBtn.click();
      await until(() => writtenRaw.length === 1, "Mobile Markdown clipboard write");
      check(writtenRaw[0] === "Tap **prose** or open [Example](https://example.com)", "Mobile Copy as Markdown writes exact source");
      await gone(() => document.querySelector('[data-message-action-bar="mobile"]'), "Mobile copy pill after copy");

      // Re-open and verify horizontal swipe drag (>10px) dismisses open pill
      wrap.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 10, pointerType: "touch", clientX: 160, clientY: 180 }));
      wrap.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, isPrimary: true, pointerId: 10, pointerType: "touch", clientX: 160, clientY: 180 }));
      await until(() => document.querySelector<HTMLElement>('[data-message-action-bar="mobile"]'), "Mobile pill open before swipe");
      wrap.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 11, pointerType: "touch", clientX: 160, clientY: 180 }));
      wrap.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, isPrimary: true, pointerId: 11, pointerType: "touch", clientX: 195, clientY: 180 }));
      await gone(() => document.querySelector('[data-message-action-bar="mobile"]'), "Mobile pill dismissed on horizontal drag");

      // Re-open and verify horizontal swipe starting on a LINK inside the bubble also dismisses pill (no lingering over drawer)
      wrap.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 12, pointerType: "touch", clientX: 160, clientY: 180 }));
      wrap.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, isPrimary: true, pointerId: 12, pointerType: "touch", clientX: 160, clientY: 180 }));
      await until(() => document.querySelector<HTMLElement>('[data-message-action-bar="mobile"]'), "Mobile pill open before link swipe");
      const linkInBubble = wrap.querySelector<HTMLAnchorElement>("a")!;
      check(Boolean(linkInBubble), "mobile bubble contains link");
      linkInBubble.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 13, pointerType: "touch", clientX: 160, clientY: 180 }));
      linkInBubble.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, isPrimary: true, pointerId: 13, pointerType: "touch", clientX: 200, clientY: 180 }));
      await gone(() => document.querySelector('[data-message-action-bar="mobile"]'), "Mobile pill dismissed when swiping from link");

      // Re-open and verify Copy + Escape dismiss
      wrap.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, pointerId: 4, pointerType: "touch", clientX: 160, clientY: 180 }));
      wrap.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, isPrimary: true, pointerId: 4, pointerType: "touch", clientX: 160, clientY: 180 }));
      const mobileBar2 = await until(() => document.querySelector<HTMLElement>('[data-message-action-bar="mobile"]'), "Mobile pill reopen");
      mobileBar2.querySelector<HTMLButtonElement>('[data-message-copy="rich"]')!.click();
      await until(() => writtenPlain.length === 1, "Mobile Copy rich/plain write");
      check(writtenPlain[0]!.includes("Tap prose or open Example (https://example.com/)"), `Mobile Copy strips Markdown (${writtenPlain[0]})`);
    } finally {
      closeMobileMessageActions();
      mobileRoot.unmount();
      host.remove();
      Object.defineProperty(navigator, "clipboard", { value: origClipboard, configurable: true });
      Object.defineProperty(globalThis, "ClipboardItem", { value: origClipboardItem, configurable: true });
    }
    passed.push("Mobile hidden message Copy/Markdown keeps zero rest chrome, preserves long-press selection, and copies from stationary tap pill");
  }

  return passed;
}
