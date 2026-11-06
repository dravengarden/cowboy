/** Composer surfaces in the real App: the schedule dialog, the delivery
 * picker's Stop confirmation, the clear-draft popper and the context
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

const SESSION = "surface-session";
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
const escape = (target: EventTarget = document.activeElement ?? document.body) =>
  target.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
const byText = (selector: string, text: string) =>
  [...document.querySelectorAll<HTMLElement>(selector)].find((node) => node.textContent?.trim() === text);
const dialogTitled = (text: string) =>
  [...document.querySelectorAll<HTMLElement>("[role=dialog]")].find((node) => node.textContent?.includes(text));

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
  // A second session gives the move-draft picker a destination.
  const other = { ...session, id: OTHER, title: "Destination session", status: "exited", updated_at_ms: now - 30_000 } as SessionMeta;
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
  await until(() => document.querySelector("[aria-label=发送时间]"), "schedule dialog");
  // A synthetic Escape is not trusted input for the Desktop key dispatcher;
  // the backdrop reaches the same onClose.
  const backdrop = await until(() => [...document.querySelectorAll<HTMLElement>(".MuiBackdrop-root")].at(-1), "schedule backdrop");
  backdrop.click();
  await gone(() => document.querySelector("[aria-label=发送时间]"), "schedule dialog");
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

  // Mobile: the clear-draft popper and the clear-conversation confirmation.
  const mobile = mount("touch", "390px");
  await typeDraft(mobile.container, "draft to clear");
  const clearButton = () => mobile.container.querySelector<HTMLElement>("[aria-label='clear composer']:not([disabled])");
  const popper = () => document.querySelector<HTMLElement>("[aria-label='Clear composer confirmation']");
  (await until(clearButton, "clear composer button")).click();
  await until(popper, "clear composer popper");
  escape();
  await gone(popper, "clear composer popper");
  check(draftText(mobile.container).includes("draft to clear"), "Escape keeps the draft");
  (await until(clearButton, "clear composer button")).click();
  const confirmClear = await until(
    () => [...(popper()?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((button) => /clear/i.test(button.textContent ?? "")),
    "clear composer confirm",
  );
  confirmClear.click();
  await gone(popper, "clear composer popper");
  await until(() => draftText(mobile.container).trim() === "", "the draft to clear");
  passed.push("Mobile clear-draft popper: Escape keeps the draft, confirming clears it");

  (await until(() => mobile.container.querySelector<HTMLElement>("[aria-label='clear conversation']:not([disabled])"), "clear conversation button")).click();
  const context = await until(() => document.querySelector<HTMLElement>("[role=dialog]"), "context confirmation");
  const cancel = byText("[role=dialog] button", "Cancel");
  check(cancel, "context confirmation cancel");
  cancel.click();
  await gone(() => context.isConnected && document.querySelector("[role=dialog]"), "context confirmation");
  check(!sent.some((frame) => frame.type === "reset" || frame.type === "clear_context"), "cancelling sends no context action");
  passed.push("Mobile clear-conversation confirmation opens and Cancel closes it without a request");

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
  return passed;
}
