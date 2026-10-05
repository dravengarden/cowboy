import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { StrictMode } from "react";
import { SessionDestinationTree } from "./SessionDestinationTree";
import type { SessionMeta } from "./protocol";
import { MobileApp } from "./mobile/MobileApp";
import { openMobileProduct } from "./mobile/appPagerMotion";
import { AppErrorBoundary } from "./AppErrorBoundary";
import { type QueuedMessage } from "./store";
import { App, CreateDialog } from "./App";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { EditorView } from "@codemirror/view";
import { undo } from "@codemirror/commands";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { DraftWorkspace } from "./documents/DraftWorkspace";
import { draftRepository } from "./documents/store";
import { useDraftRoute } from "./documents/navigation";
import {
  type DraftDocument,
  draftMetadata,
  type DraftMutationArgs,
  expectedRevision,
  projectDraft,
} from "./documents/model";
import {
  activeEditorExtensionPort,
  closeEditorExtensions,
  openEditorExtensions,
} from "./editorExtensions/host";
import {
  EditorExtensionsCommand,
  EditorExtensionsDialog,
} from "./editorExtensions/EditorExtensionsDialog";
import { editorPluginHost } from "./editorPlugins/appHost";
import { DocumentNotifications } from "./documents/DocumentNotifications";
import { isMac } from "./platform";

/** Visible operable controls without any keyboard slot (FOCUS.md "Leader").
 *  A control counts as keyboard-reachable when it shows a keycap, declares
 *  aria-keyshortcuts, is a navigable list item (item verbs), lives inside an
 *  editor, or sits in a `data-desktop-shortcut-owner` whose one slot drives
 *  the whole group (the History/Explore toggle). */
export function shortcutAudit(root: Document): string[] {
  const covered = (element: HTMLElement): boolean => {
    const owner = element.closest("[data-desktop-shortcut-owner]");
    return !!owner?.querySelector("[data-shortcut-state], kbd");
  };
  const controls = [
    ...root.querySelectorAll<HTMLElement>(
      "button, [role=button], [role=tab], [role=menuitem], [role=switch], a[href]",
    ),
  ].filter((element) =>
    element.getClientRects().length > 0 &&
    !element.matches(":disabled, [aria-disabled=true]") &&
    !element.closest("[aria-hidden=true], .cm-editor, [data-desktop-item]") &&
    !element.querySelector("[data-shortcut-state], kbd") &&
    !element.closest("[data-shortcut-state]") &&
    !element.hasAttribute("aria-keyshortcuts") &&
    !covered(element)
  );
  return controls.map((element) => {
    const name = (element.getAttribute("aria-label") || element.title ||
      element.textContent || element.tagName).replace(/\s+/g, " ").trim()
      .slice(0, 60);
    const region = element.closest<HTMLElement>("[data-desktop-region]")
      ?.dataset.desktopRegion ??
      element.closest<HTMLElement>("[data-desktop-pane]")?.dataset.desktopPane ??
      (element.closest("[data-desktop-status-line]") ? "status-line" : "chrome");
    return `${region} :: ${name}`;
  });
}

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
const tick = (ms = 60) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
async function until(predicate: () => boolean, label: string): Promise<void> {
  for (let i = 0; i < 120; i++) {
    if (predicate()) return;
    await tick();
  }
  throw new Error(
    `Timed out: ${label}; ${
      document.body.textContent?.slice(-1200)
    }; hash=${location.hash}`,
  );
}
function button(label: string): HTMLElement {
  const element = [
    ...document.querySelectorAll<HTMLElement>("button,[role=button]"),
  ].find((item) =>
    item.getClientRects().length &&
    (item.getAttribute("aria-label") === label ||
      item.textContent?.trim() === label ||
      (label.startsWith("Create ") &&
        item.textContent?.trim().startsWith(label)))
  );
  check(element, `Missing button: ${label}`);
  return element;
}

/** Real Draft workspace, shared editor/extension host and dataset-owned IndexedDB.
 * Only remote HTTP is a fixture. Native keyboard acceptance is a separate run. */
export async function runDraftDocumentsBrowserConformance(
  nativeMode = false,
): Promise<string[]> {
  const originalFetch = globalThis.fetch;
  const originalWebSocket = globalThis.WebSocket;
  const device = (globalThis as unknown as {
    CowboyDeviceProof: { proof: (url: string) => Promise<string> };
  }).CowboyDeviceProof;
  const originalProof = device.proof;
  device.proof = () => Promise.resolve("isolated-fixture");
  let socket: FixtureSocket | undefined;
  let sharedFolder = "";
  let copied: (QueuedMessage & { content: unknown })[] = [];
  let queueVersion = 0;
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
          type: "sessions",
          sessions: nativeMode
            ? [{
              id: "native-session",
              title: "Session",
              provider: "codex",
              cwd: "/fixture",
              status: "running",
            }]
            : [],
        });
        this.publish({
          type: "sync_patch",
          state: "folders",
          version: 0,
          value: {
            folders: [{
              id: sharedFolder,
              name: "Research",
              parent: null,
              project: null,
              position: 0,
            }],
            placement: {},
          },
          confirmed: [],
          resync: true,
        });
        this.publish({
          type: "sync_patch",
          state: "workspace-order",
          version: 0,
          value: [],
          confirmed: [],
          resync: true,
        });
        this.publish({ type: "bootstrap_complete" });
      }, 20);
    }
    publish(message: unknown) {
      this.onmessage?.({ data: JSON.stringify(message) });
    }
    send(data: string) {
      const command = JSON.parse(data);
      if (command.type === "add_draft") {
        copied.push({
          id: `server-${command.cmid}`,
          cmid: command.cmid,
          text: command.text,
          attachments: [],
          content: command.content,
        });
        setTimeout(
          () =>
            this.publish({
              type: "sync_patch",
              state: `queue:${command.session_id}`,
              version: ++queueVersion,
              value: { queue: [], drafts: copied, inFlight: [] },
              confirmed: [command.cmid],
            }),
          10,
        );
      }
    }
    close() {
      this.readyState = 3;
    }
  }
  globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;
  const originalMatchMedia = globalThis.matchMedia;
  // Headless Firefox defaults to pointer:none. Exercise the actual Desktop
  // product branch while keeping the real viewport/theme media queries.
  let touchCreate = false;
  globalThis.matchMedia = (query) => {
    const pointer = query.includes("(pointer: fine)") ||
      query.includes("(hover: hover)");
    const coarse = query.includes("(any-pointer: coarse)");
    if (touchCreate && /max-width: (599|1199)/.test(query)) {
      return {
        ...originalMatchMedia.call(globalThis, query),
        matches: true,
        addListener() {},
        removeListener() {},
        addEventListener() {},
        removeEventListener() {},
      };
    }
    if (!pointer && !coarse) return originalMatchMedia.call(globalThis, query);
    return {
      matches: touchCreate ? coarse : pointer,
      media: query,
      onchange: null,
      addListener() {},
      removeListener() {},
      addEventListener() {},
      removeEventListener() {},
      dispatchEvent: () => true,
    };
  };

  const originalFont = document.documentElement.style.fontSize;
  const server = new Map<string, DraftDocument>();
  const operations = new Set<string>();
  let offline = false;
  check(bindProductSyncPrincipal("draft-fixture"), "fixture principal bound");
  globalThis.fetch = (input, init) => {
    const path = String(input);
    if (!path.startsWith("/api/")) return originalFetch(input, init);
    if (offline) return Promise.reject(new Error("Fixture offline"));
    if (path === "/api/sync/dataset") {
      return Promise.resolve(Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"a".repeat(64)}`,
        user_id: "draft-fixture",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      }));
    }
    if (path.includes("/draft-copies/") && init?.method === "DELETE") {
      copied = copied.filter((row) => row.cmid !== path.split("/").at(-1));
      socket?.publish({
        type: "sync_patch",
        state: "queue:integrated-session",
        version: ++queueVersion,
        value: { queue: [], drafts: copied, inFlight: [] },
        confirmed: [],
      });
      return Promise.resolve(new Response(null, { status: 204 }));
    }
    if (path.endsWith("/bootstrap")) {
      const session = path.split("/")[3]!;
      return Promise.resolve(Response.json({
        messages: [
          {
            type: "snapshot",
            session_id: session,
            events: [],
            reached_start: true,
          },
          {
            type: "sync_patch",
            state: `queue:${session}`,
            version: queueVersion,
            value: { queue: [], drafts: copied, inFlight: [] },
            confirmed: [],
            resync: true,
          },
        ],
      }));
    }
    if (path === "/api/drafts") {
      return Promise.resolve(
        Response.json({ entries: [...server.values()].map(draftMetadata) }),
      );
    }
    if (path === "/api/drafts/mutations") {
      const args = JSON.parse(String(init?.body)) as DraftMutationArgs & {
        operation_id: string;
      };
      const old = server.get(args.document_id) ?? null;
      if (operations.has(args.operation_id)) {
        return Promise.resolve(Response.json(old));
      }
      if (expectedRevision(old, args.change) !== args.expected_revision) {
        return Promise.resolve(
          Response.json({
            current: old,
            error:
              "This draft changed elsewhere. Your local version has been kept.",
          }, { status: 409 }),
        );
      }
      const next = projectDraft(old, { ...args, authored_at_ms: Date.now() })!;
      server.set(next.id, next);
      operations.add(args.operation_id);
      return Promise.resolve(Response.json(next));
    }
    if (path.endsWith("/history")) return Promise.resolve(Response.json([]));
    const row = server.get(path.split("/").at(-1)!);
    return Promise.resolve(
      row ? Response.json(row) : Response.json({}, { status: 404 }),
    );
  };
  const container = document.createElement("div");
  container.style.cssText = "width:1200px;height:800px;position:relative";
  document.body.append(container);
  const root = createRoot(container);
  const results: string[] = [];
  const repo = draftRepository();
  function Harness() {
    const route = useDraftRoute();
    return (
      <>
        <DraftWorkspace id={route.id} />
        <EditorExtensionsDialog />
        <DocumentNotifications />
      </>
    );
  }
  try {
    if (nativeMode) globalThis.matchMedia = originalMatchMedia;
    await repo.start();
    const folder = await repo.create("Research", null, "folder");
    sharedFolder = folder;
    // Store bootstrap can connect before the async repository creates Research.
    // Publish the final fixture folder identity rather than relying on timer order.
    socket?.publish({
      type: "sync_patch",
      state: "folders",
      version: 1,
      value: {
        folders: [{
          id: folder,
          name: "Research",
          parent: null,
          project: null,
          position: 0,
        }],
        placement: {},
      },
      confirmed: [],
      resync: true,
    });
    if (nativeMode) {
      const id = await repo.create("Native Draft", folder, "document", "");
      globalThis.location.hash = `drafts/${id}`;
      container.style.cssText = "height:100dvh;width:100%;position:relative";
      document.body.style.margin = "0";
      root.render(
        <SurfaceProvider>
          <BrowserProductTheme>
            <CssBaseline />
            <AppErrorBoundary>
              <MobileApp themeMode="light" onSetThemeMode={() => {}} />
            </AppErrorBoundary>
            <DocumentNotifications />
          </BrowserProductTheme>
        </SurfaceProvider>,
      );
      const events: { type: string; data: string | null }[] = [];
      for (
        const type of [
          "compositionstart",
          "compositionupdate",
          "compositionend",
          "beforeinput",
        ]
      ) {
        document.addEventListener(type, (event) => {
          events.push({ type, data: (event as InputEvent).data ?? null });
          if (events.length > 120) events.shift();
        });
      }
      setInterval(() => {
        const draft = repo.document(id).get().document;
        const input = document.querySelector<HTMLTextAreaElement>(
          "textarea[data-mobile-native-textarea]",
        );
        const bridge = (globalThis as unknown as {
          webkit?: {
            messageHandlers: {
              report: { postMessage: (value: unknown) => void };
            };
          };
        }).webkit;
        bridge?.messageHandlers.report.postMessage({
          value: input?.value ?? draft?.body,
          saved: draft?.body,
          native: !!input,
          context: activeEditorExtensionPort()?.context,
          hash: location.hash,
          focus: document.activeElement === input,
          selection: input
            ? { anchor: input.selectionStart, head: input.selectionEnd }
            : null,
          events,
          product: document.querySelector("[data-mobile-product]")
            ?.getAttribute("data-mobile-product"),
          sidebarRows: document.querySelectorAll("[data-desktop-item]").length,
        });
      }, 200);
      await new Promise<void>(() => {});
    }
    // The actual Create dialog must not gate Draft creation on Machine/AI
    // readiness. Exercise Desktop and touch with an empty Machine store.
    for (const touch of [false, true]) {
      touchCreate = touch;
      let closed = false;
      flushSync(() =>
        root.render(
          <StrictMode>
            <SurfaceProvider>
              <BrowserProductTheme>
                <CssBaseline />
                <DesktopWorkspaceProvider>
                  <DesktopCommandProvider>
                    <CreateDialog
                      initialFolder={folder}
                      open
                      onClose={() => {
                        closed = true;
                      }}
                      onCreated={() => {
                        throw new Error("Draft must not create a Session");
                      }}
                    />
                  </DesktopCommandProvider>
                </DesktopWorkspaceProvider>
              </BrowserProductTheme>
            </SurfaceProvider>
          </StrictMode>,
        )
      );
      await tick(180);
      const tab = (name: string) => {
        const item = [...document.querySelectorAll<HTMLElement>("[role=tab]")]
          .find((element) => element.getAttribute("aria-label") === name);
        check(item, `Create variant ${name}`);
        item.focus();
        return item;
      };
      check(
        button("Create session").hasAttribute("disabled"),
        "No Machine disables Session only",
      );
      if (!touch) {
        // Desktop Vim layers. Letters arrive as a CJK input source reports
        // them (`Process`) so only physical codes can resolve them.
        const press = (
          target: Element,
          code: string,
          init: KeyboardEventInit = {},
        ): void => {
          target.dispatchEvent(
            new KeyboardEvent("keydown", {
              code,
              key: "Process",
              bubbles: true,
              cancelable: true,
              ...init,
            }),
          );
        };
        const selected = () =>
          document.querySelector<HTMLElement>("[role=tab][aria-selected=true]")
            ?.getAttribute("aria-label");
        const sessionTitle = document.querySelector<HTMLInputElement>("input");
        check(
          sessionTitle && document.activeElement === sessionTitle,
          "Create opens in the title field",
        );
        // The shared modal grammar (FOCUS.md "Modals").
        check(
          document.querySelector('[data-create-key-hint="text"]') &&
            document.querySelectorAll("[role=tab] [data-shortcut-state='inactive']").length === 3,
          "Insert advertises Esc; the type digits wait for Normal",
        );
        press(sessionTitle, "Escape", { key: "Escape", isComposing: true });
        await tick();
        check(
          !closed && document.activeElement === sessionTitle,
          "Esc during composition stays with the IME",
        );
        press(sessionTitle, "Escape", { key: "Escape" });
        await tick();
        check(!closed, "Esc in the title does not close Create");
        const cursor = () => document.activeElement as HTMLElement;
        check(
          cursor().hasAttribute("data-desktop-field-cursor") &&
            cursor().contains(sessionTitle),
          "Esc leaves Insert with the Normal cursor on the title",
        );
        check(
          document.querySelectorAll("[role=tab] [data-shortcut-state='available']").length === 3,
          "Normal lights the 1–3 type digits",
        );
        press(cursor(), "Digit2");
        await tick();
        check(
          selected() === "Draft" && cursor().hasAttribute("data-desktop-field-cursor"),
          "2 picks Draft and the cursor stays on the title",
        );
        press(cursor(), "KeyL");
        await tick();
        check(selected() === "Folder", "l on a single-field row steps the tabs");
        press(cursor(), "KeyH");
        await tick();
        check(selected() === "Draft", "h steps back");
        press(cursor(), "Digit1");
        await tick();
        check(selected() === "Session", "1 returns to Session");
        press(cursor(), "KeyK");
        await tick();
        check(
          cursor().getAttribute("aria-label") === "Session",
          "k moves up to the selected tab",
        );
        press(cursor(), "KeyL");
        await tick();
        check(
          selected() === "Draft" && cursor().getAttribute("aria-label") === "Draft",
          "l on the tab row selects and follows the next tab",
        );
        press(cursor(), "BracketLeft", { key: "[" });
        await tick();
        check(selected() === "Session", "[ steps the tabs back");
        press(cursor(), "BracketRight", { key: "]" });
        await tick();
        check(selected() === "Draft", "] steps the tabs forward");
        press(cursor(), "KeyJ");
        await tick();
        check(
          cursor().hasAttribute("data-desktop-field-cursor"),
          "j moves down to the title",
        );
        press(cursor(), "KeyJ");
        await tick();
        check(
          !cursor().hasAttribute("data-desktop-field-cursor") &&
            cursor().closest("[role='dialog']") !== null,
          "j continues to the next control",
        );
        press(cursor(), "KeyG", { key: "G", shiftKey: true });
        await tick();
        check(
          cursor().textContent?.includes("Create draft"),
          "G reaches the last control, the confirm button",
        );
        press(cursor(), "KeyH");
        await tick();
        check(
          cursor().textContent?.includes("Cancel"),
          "h moves across the button row",
        );
        press(cursor(), "KeyG", { key: "g" });
        press(cursor(), "KeyG", { key: "g" });
        await tick();
        check(
          cursor().getAttribute("role") === "tab",
          "gg returns to the first control, the tabs",
        );
        // Back to Session before editing: the Draft title's first focus
        // consumes its one-time name selection, verified below.
        press(cursor(), "Digit1");
        await tick();
        check(
          selected() === "Session" && cursor().getAttribute("aria-label") === "Session",
          "A digit on the tab row moves the cursor with the tab",
        );
        press(cursor(), "KeyJ");
        await tick();
        press(cursor(), "KeyI");
        await tick();
        const keyboardTitle = document.querySelector<HTMLInputElement>("input");
        check(
          keyboardTitle && document.activeElement === keyboardTitle,
          "i edits the field under the cursor",
        );
        press(keyboardTitle, "BracketLeft", { key: "[", ctrlKey: true });
        await tick();
        check(
          !closed && cursor().hasAttribute("data-desktop-field-cursor"),
          "Ctrl-[ is the Vim Esc alias",
        );
        press(cursor(), "Escape", { key: "Escape" });
        await tick();
        check(closed, "Esc in Normal closes Create");
        closed = false;
        press(cursor(), "Enter", { key: "Enter" });
        await tick();
        check(
          document.activeElement === document.querySelector("input"),
          "Enter on the title cursor edits it",
        );
        tab("Session").click();
        await tick();
      }
      tab("Draft").click();
      await tick();
      const title = document.querySelector<HTMLInputElement>("input");
      check(title, "Draft title field");
      check(
        /^Draft \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(title.value),
        "Draft starts with a local date/time name",
      );
      check(
        document.activeElement === title,
        "Selecting Draft automatically focuses Title",
      );
      check(
        title.selectionStart === 0 && title.selectionEnd === title.value.length,
        "Selecting Draft automatically selects the generated name for replacement",
      );
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!;
      setter.call(title, "独立 draft 🌏");
      title.dispatchEvent(new Event("input", { bubbles: true }));
      await tick();
      check(
        ![...document.querySelectorAll("label")].some((label) =>
          label.textContent === "AI installation"
        ),
        "Draft hides AI configuration",
      );
      check(
        document.body.textContent?.includes("Directory (optional)"),
        "Draft shares the optional workspace directory",
      );
      check(
        !button("Create draft").hasAttribute("disabled"),
        "Draft creates without a Machine",
      );
      const directoryInput = document.querySelector<HTMLInputElement>(
        'input[role="combobox"]',
      );
      check(
        directoryInput?.value === "Research",
        "Directory-context creation carries its folder into Draft",
      );
      directoryInput.click();
      await tick();
      const folderChoice = [
        ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
      ]
        .find((item) => item.textContent?.includes("Research"));
      check(folderChoice, "Existing Draft folder is selectable");
      folderChoice.click();
      await tick();
      if (touch) {
        button("Clear Directory (optional)").click();
        await tick();
      }
      tab("Session").click();
      await tick();
      tab("Draft").click();
      await tick();
      check(
        document.querySelector<HTMLInputElement>("input")?.value ===
          "独立 draft 🌏",
        "Switching variants preserves entered title",
      );
      const editedTitle = document.querySelector<HTMLInputElement>("input")!;
      editedTitle.focus();
      editedTitle.setSelectionRange(2, 2);
      editedTitle.blur();
      editedTitle.focus();
      await tick();
      check(
        editedTitle.selectionStart === 2 && editedTitle.selectionEnd === 2,
        "Returning to a custom title preserves the caret",
      );
      // IME candidate confirmation must never create either variant.
      document.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          ctrlKey: true,
          isComposing: true,
          bubbles: true,
        }),
      );
      await tick();
      check(!closed, "IME candidate does not create a Draft");
      const countBefore = repo.get().entries.length;
      offline = true;
      button("Create draft").click();
      button("Create draft").click();
      await until(() => closed, "local-first Draft created offline");
      const createdId = globalThis.location.hash.split("/")[1];
      check(
        createdId &&
          repo.document(createdId).get().document?.title === "独立 draft 🌏",
        "Create opens the persisted Draft",
      );
      check(
        repo.document(createdId).get().document?.parent_id ===
          (touch ? null : folder),
        "Draft folder survives variant switch; cleared selection creates at root",
      );
      check(
        repo.get().entries.length === countBefore + 1,
        "Double tap creates only one Draft",
      );
      offline = false;
      await repo.document(createdId).retry();
      await repo.document(createdId).whenSynced();
      flushSync(() => root.render(<></>));
      await tick();
    }
    touchCreate = false;
    results.push(
      "Unified Create offers Session/Draft on Desktop and touch; no Machine required for Draft, retained title, IME guard, offline persistence, Draft folders and clear-to-root placement",
    );
    const id = await repo.create(
      "独立笔记",
      folder,
      "document",
      "# Heading\n\n你好 🌏\n",
    );
    await repo.document(id).whenSynced();
    globalThis.location.hash = `drafts/${id}`;
    flushSync(() =>
      root.render(
        <StrictMode>
          <SurfaceProvider>
            <BrowserProductTheme>
              <CssBaseline />
              <Harness />
            </BrowserProductTheme>
          </SurfaceProvider>
        </StrictMode>,
      )
    );
    await until(
      () => activeEditorExtensionPort()?.context.kind === "document",
      "shared editor registered after StrictMode",
    );
    check(
      container.querySelector(".cm-editor"),
      "Actual shared CodeMirror editor",
    );
    check(
      !container.querySelector("[data-machine-setup-gate], [data-code-pane]"),
      "No Machine or Code surface",
    );
    const row = container.querySelector<HTMLElement>("[role=treeitem]");
    check(row, "Draft tree is present");
    for (
      const modifiers of [{ ctrlKey: true }, { metaKey: true }, {
        altKey: true,
      }]
    ) {
      const key = new KeyboardEvent("keydown", {
        key: "j",
        bubbles: true,
        cancelable: true,
        ...modifiers,
      });
      row.dispatchEvent(key);
      check(!key.defaultPrevented, "Tree preserves modified browser shortcuts");
    }
    const port = activeEditorExtensionPort()!;
    check(
      port.context.surface === "desktop",
      "Actual Desktop surface is under test",
    );
    check(
      port.context.id === id && port.read().text.includes("你好 🌏"),
      "Independent document identity/text",
    );
    check(port.replaceSelection("中文输入 📝\n", port.read()), "Edit accepted");
    await until(
      () => server.get(id)?.body.includes("中文输入 📝") === true,
      "authored unicode persisted",
    );
    results.push(
      "Independent machine-free Draft mounts the actual shared editor under StrictMode and persists Unicode through IndexedDB",
    );

    button("Editor extensions").click();
    await until(
      () =>
        [...document.querySelectorAll("[role=button]")].some((e) =>
          e.textContent === "Plan"
        ),
      "extension commands",
    );
    button("Plan").click();
    await until(
      () => port.read().text.includes("## Verification"),
      "template insertion",
    );
    const content = container.querySelector<HTMLElement>(".cm-content")!;
    const view = EditorView.findFromDOM(content)!;
    check(undo(view), "Template uses normal editor undo");
    check(
      !port.read().text.includes("## Verification"),
      "Undo restores exact pre-template document",
    );
    content.dispatchEvent(
      new CompositionEvent("compositionstart", {
        bubbles: true,
        data: "zhong",
      }),
    );
    check(
      !port.replaceSelection("forbidden", port.read()),
      "Plugin is fenced during native composition",
    );
    content.dispatchEvent(
      new CompositionEvent("compositionend", { bubbles: true, data: "" }),
    );
    await tick(150);
    results.push(
      "Shared template plugin preserves editor undo and refuses composition-time writes",
    );

    button("Move draft").click();
    await tick();
    const top = [...document.querySelectorAll<HTMLElement>("[role=button]")]
      .find((e) => e.textContent?.includes("Drafts · top level"));
    check(top, "Explicit no-directory destination");
    top.click();
    await until(
      () => server.get(id)?.parent_id === null,
      "move to root committed",
    );
    check(
      server.get(id)?.body.includes("中文输入"),
      "Moving preserves content",
    );
    results.push(
      "Directory picker moves documents to top level without changing their content or identity",
    );

    offline = true;
    check(
      port.replaceSelection("离线保存\n", port.read()),
      "Offline edit accepted",
    );
    await until(
      () => repo.document(id).get().phase === "local",
      "durable offline state",
    );
    check(
      repo.document(id).get().document?.body.includes("离线保存"),
      "Offline authored branch retained",
    );
    offline = false;
    await repo.document(id).retry();
    await repo.document(id).whenSynced();
    check(server.get(id)?.body.includes("离线保存"), "Offline outbox replayed");
    results.push(
      "Browser IndexedDB retains offline edits and replays them on reconnect",
    );

    const before = server.get(id)!;
    server.set(id, {
      ...before,
      body: `Remote writer\n\n${before.body}`,
      revision: before.revision + 1,
      body_revision: before.body_revision + 1,
    });
    // Typed here before this device has seen the other writer.
    check(
      port.replaceSelection("My competing edit\n", port.read()),
      "Local edit while another device writes",
    );
    repo.announce({ [id]: draftMetadata(server.get(id)!) });
    await until(
      () =>
        server.get(id)?.body.includes("My competing edit") === true &&
        repo.document(id).get().phase === "saved",
      "concurrent edits merged",
    );
    check(
      server.get(id)?.body.startsWith("Remote writer") &&
        port.read().text === server.get(id)?.body,
      "Both writers survive in the server and the open editor",
    );
    check(
      !document.body.textContent?.includes("changed elsewhere"),
      "Merging shows no conflict banner",
    );
    results.push(
      "Concurrent writers merge into the open editor without a conflict or copy",
    );

    const recovered = activeEditorExtensionPort()!;
    const longText = Array.from(
      { length: 1500 },
      (_, i) => `Paragraph ${i}: Markdown 中文 content, no layout widgets.\n\n`,
    ).join("");
    recovered.reveal(0);
    const began = performance.now();
    check(
      recovered.replaceSelection(longText, recovered.read()),
      "Long document insertion",
    );
    await tick();
    const duration = performance.now() - began;
    check(
      container.querySelectorAll(".cm-line").length < 300,
      "Long document keeps viewport rendering",
    );
    check(
      duration < 5000,
      `Long document update took ${duration.toFixed(0)}ms`,
    );
    results.push(
      `Long Markdown stays viewport-rendered (${longText.length} characters; insertion ${
        duration.toFixed(0)
      }ms in this browser)`,
    );

    for (const font of [8, 10.4, 16, 24, 32]) {
      for (const width of [320, 600, 960, 1440]) {
        document.documentElement.style.fontSize = `${font}px`;
        container.style.width = `${width}px`;
        await tick(100);
        const workspace = container.querySelector<HTMLElement>(
          "[data-draft-workspace]",
        )!;
        check(
          workspace.scrollWidth <= workspace.clientWidth + 1,
          `Workspace overflow at ${width}px/${font}px: ${workspace.scrollWidth}`,
        );
        for (
          const action of container.querySelectorAll<HTMLElement>(
            "button[data-draft-tool]",
          )
        ) {
          if (!action.getClientRects().length) continue;
          const icon = action.querySelector("svg")!;
          const style = getComputedStyle(action);
          check(
            icon.getBoundingClientRect().width + parseFloat(style.paddingLeft) +
                parseFloat(style.paddingRight) <=
              action.getBoundingClientRect().width + 1,
            "Desktop toolbar icon and padding follow the global font size",
          );
        }
        for (
          const label of [
            "Copy to Session drafts",
            "Recovery history",
            "Export Markdown",
            "Attach file",
          ]
        ) {
          const button = container.querySelector<HTMLElement>(
            `button.MuiIconButton-root[aria-label="${label}"]`,
          )!;
          if (!button?.getClientRects().length) continue;
          const svg = button.querySelector("svg")!;
          check(
            Math.abs(button.getBoundingClientRect().width - 2.25 * font) < 1,
            `${label} target scales with root font`,
          );
          check(
            Math.abs(svg.getBoundingClientRect().width - 1.5 * font) < 1,
            `${label} glyph scales with root font`,
          );
        }
        const formatBar = container.querySelector<HTMLElement>(
          "[data-draft-format-toolbar]",
        )!;
        check(
          formatBar.scrollWidth <= formatBar.clientWidth + 1,
          "Grouped formatting controls wrap within the available width",
        );
        const editor = container.querySelector<HTMLElement>(".cm-editor")!;
        check(
          editor.getBoundingClientRect().height > 200,
          `Editing canvas retains useful height at ${width}px/${font}px: ${editor.getBoundingClientRect().height}`,
        );
      }
    }
    results.push(
      "Actual Desktop workspace, tabs and toolbar fit 320–1440px at 8–32px fonts; modified browser shortcuts are preserved",
    );
    await repo.document(recovered.context.id).whenSynced();
    flushSync(() => root.unmount());
    check(
      !recovered.replaceSelection("late plugin", recovered.read()),
      "Unmounted editor rejects stale extension write",
    );
    results.push(
      "Unmount revokes old plugin editor authority and keeps the saved document",
    );
    document.documentElement.style.fontSize = "16px";
    const destinations = createRoot(container);
    let chosen = "";
    const destinationSessions: SessionMeta[] = [
      {
        id: "tree-first",
        title: "Same title",
        provider: "codex",
        cwd: "/first",
        status: "running",
      },
      {
        id: "tree-second",
        title: "Same title",
        provider: "codex",
        cwd: "/second",
        status: "running",
      },
      {
        id: "tree-root",
        title: "Root Session",
        provider: "codex",
        cwd: "/root",
        status: "running",
      },
      {
        id: "tree-other",
        title: "Other Session",
        provider: "codex",
        cwd: "/other",
        status: "running",
      },
      {
        id: "tree-system",
        title: "System",
        provider: "codex",
        cwd: "/system",
        status: "running",
        system: true,
      },
    ];
    const renderDestinations = (busy = false) =>
      flushSync(() =>
        destinations.render(
          <SurfaceProvider>
            <BrowserProductTheme>
              <CssBaseline />
              <SessionDestinationTree
                sessions={destinationSessions}
                folders={{
                  folders: [
                    {
                      id: "tree-parent",
                      name: "Parent",
                      parent: null,
                      position: 0,
                      project: null,
                    },
                    {
                      id: "tree-child",
                      name: "Child",
                      parent: "tree-parent",
                      position: 0,
                      project: null,
                    },
                    {
                      id: "tree-sibling",
                      name: "Sibling",
                      parent: null,
                      position: 1,
                      project: null,
                    },
                  ],
                  placement: {
                    "tree-first": "tree-child",
                    "tree-second": "tree-child",
                    "tree-other": "tree-sibling",
                  },
                }}
                order={["session:tree-second", "session:tree-first"]}
                initialFolder="tree-child"
                busy={busy}
                onPick={(session) => {
                  chosen = session.id;
                }}
              />
            </BrowserProductTheme>
          </SurfaceProvider>,
        )
      );
    renderDestinations();
    await tick();
    const destinationFolder = (id: string) =>
      container.querySelector<HTMLElement>(
        `[data-session-destination-folder="${id}"]`,
      )!;
    const destinationSession = (id: string) =>
      container.querySelector<HTMLElement>(
        `[data-session-destination-session="${id}"]`,
      );
    check(
      destinationFolder("tree-parent").getAttribute("aria-expanded") ===
          "true" &&
        destinationFolder("tree-child").getAttribute("aria-expanded") ===
          "true",
      "Draft context opens both ancestor levels",
    );
    check(
      destinationFolder("tree-sibling").getAttribute("aria-expanded") ===
          "false" && !destinationSession("tree-other"),
      "Other directories start folded",
    );
    check(
      destinationSession("tree-root") && !destinationSession("tree-system"),
      "Top-level Sessions are available; system Sessions excluded",
    );
    check(
      [...container.querySelectorAll<HTMLElement>(
        "[data-session-destination-session]",
      )].slice(0, 2).map((row) => row.dataset.sessionDestinationSession)
        .join() === "tree-second,tree-first",
      "Shared workspace order distinguishes same-title Sessions by identity",
    );
    destinationFolder("tree-parent").click();
    await tick();
    check(
      !destinationSession("tree-first"),
      "Collapsing an ancestor hides descendant leaves without selecting anything",
    );
    const destinationSearch = container.querySelector<HTMLInputElement>(
      "input",
    )!;
    const searchDestination = async (value: string) => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!
        .call(destinationSearch, value);
      destinationSearch.dispatchEvent(new Event("input", { bubbles: true }));
      await tick();
    };
    await searchDestination("Child");
    check(
      destinationSession("tree-first") && !destinationSession("tree-root") &&
        !destinationFolder("tree-sibling"),
      "Search includes matching folder paths and their ancestors",
    );
    await searchDestination("");
    check(
      !destinationSession("tree-first") &&
        destinationFolder("tree-parent").getAttribute("aria-expanded") ===
          "false",
      "Clearing search restores local folds",
    );
    const destinationKey = (target: HTMLElement, key: string) =>
      target.dispatchEvent(
        new KeyboardEvent("keydown", { key, bubbles: true }),
      );
    destinationFolder("tree-parent").focus();
    destinationKey(destinationFolder("tree-parent"), "ArrowRight");
    await tick();
    destinationSession("tree-second")!.focus();
    destinationKey(destinationSession("tree-second")!, "ArrowLeft");
    check(
      document.activeElement === destinationFolder("tree-child"),
      "Left arrow moves from a Session to its parent folder",
    );
    destinationSession("tree-second")!.focus();
    destinationSession("tree-second")!.dispatchEvent(
      new KeyboardEvent("keydown", {
        bubbles: true,
        key: "Enter",
        isComposing: true,
        keyCode: 229,
      }),
    );
    check(!chosen, "IME Enter cannot select a Session");
    destinationSession("tree-second")!.click();
    check(
      chosen === "tree-second",
      "Same-title Session picks its exact identity",
    );
    renderDestinations(true);
    await tick();
    destinationSession("tree-first")!.click();
    check(chosen === "tree-second", "Busy copy blocks another selection");
    renderDestinations();
    await tick();
    for (const font of [8, 16, 24]) {
      document.documentElement.style.fontSize = `${font}px`;
      container.style.width = "320px";
      await tick();
      check(
        container.scrollWidth <= container.clientWidth + 1,
        "Tree fits narrow panes at global font sizes",
      );
    }
    destinations.unmount();
    results.push(
      "Session destinations use a shared ordered directory tree; context expansion, folds/search, root leaves, duplicate identities, keyboard/IME guards, busy selection and font fit pass",
    );
    document.documentElement.style.fontSize = "16px";
    container.style.cssText =
      "width:1200px;height:800px;position:relative;display:flex";
    globalThis.location.hash = `drafts/${id}`;
    const integrated = createRoot(container);
    flushSync(() =>
      integrated.render(
        <StrictMode>
          <SurfaceProvider>
            <BrowserProductTheme>
              <CssBaseline />
              <AppErrorBoundary>
                <DesktopWorkspaceProvider>
                  <DesktopCommandProvider>
                    <EditorExtensionsCommand />
                    <App
                      themeMode="light"
                      onSetThemeMode={() => {}}
                      surface="desktop"
                    />
                    <EditorExtensionsDialog />
                  </DesktopCommandProvider>
                </DesktopWorkspaceProvider>
              </AppErrorBoundary>
              <DocumentNotifications />
            </BrowserProductTheme>
          </SurfaceProvider>
        </StrictMode>,
      )
    );
    await until(
      () => !!container.querySelector("[data-workspace-document]"),
      "Draft opens in the ordinary App",
    );
    await until(
      () => !!container.querySelector(`[data-desktop-item="draft:${id}"]`),
      "Draft appears in the shared folder tree",
    );
    check(
      ![...container.querySelectorAll("button")].some((item) =>
        item.textContent?.trim() === "Drafts"
      ),
      "No standalone Drafts mode entry",
    );
    const rail = container.querySelector(
      '[data-desktop-region="sessions.list"]',
    );
    check(rail, "Draft keeps the ordinary workspace sidebar mounted");
    await until(() => !!container.querySelector("[data-desktop-draft-toolbar]"), "Integrated Draft mounts keyboard-first Desktop controls");
    const originalScale = document.documentElement.style.getPropertyValue("--cowboy-font-scale");
    for (const size of [8, 16, 24, 32]) {
      document.documentElement.style.fontSize = `${size}px`;
      document.documentElement.style.setProperty("--cowboy-font-scale", String(size / 16));
      await tick();
      const create = container.querySelector<HTMLElement>("[data-desktop-new-session]")!;
      check(Math.abs(parseFloat(getComputedStyle(create).fontSize) - size * 0.875) < 1, "Sidebar Create label tracks root font");
      const toolbar = create.parentElement!;
      check(toolbar.scrollWidth <= toolbar.clientWidth + 1, `Sidebar Create/fold controls fit at ${size}px`);
    }
    document.documentElement.style.fontSize = originalFont;
    if (originalScale) document.documentElement.style.setProperty("--cowboy-font-scale", originalScale);
    else document.documentElement.style.removeProperty("--cowboy-font-scale");
    const integratedBody = repo.document(id).get().document!.body;
    socket!.publish({
      type: "sessions",
      sessions: [{
        id: "integrated-session",
        title: "Real session row",
        provider: "codex",
        cwd: "/fixture",
        status: "running",
      }],
    });
    await until(
      () =>
        !!container.querySelector('[data-desktop-item="integrated-session"]'),
      "Mixed Session and Draft entries",
    );
    container.querySelector<HTMLElement>(
      '[data-desktop-item="integrated-session"]',
    )!.click();
    await until(
      () => !container.querySelector("[data-workspace-document]"),
      "Session selects the ordinary conversation",
    );
    check(
      container.querySelector('[data-desktop-region="sessions.list"]') === rail,
      "Switching types preserves the sidebar DOM and folds",
    );
    container.querySelector<HTMLElement>(`[data-desktop-item="draft:${id}"]`)!
      .click();
    await until(
      () => !!container.querySelector("[data-workspace-document]"),
      "Draft reopens in place",
    );
    await until(
      () => activeEditorExtensionPort()?.context.kind === "document",
      "Shared editor restored",
    );
    check(
      repo.document(id).get().document!.body === integratedBody,
      "Switching to Session and back preserves exact document content",
    );
    // Exercise the production grip path: center drop copies, source survives,
    // and snackbar Undo removes only its exact unsent copy.
    const draftRow = container.querySelector<HTMLElement>(
      `[data-desktop-item="draft:${id}"]`,
    )!;
    const sessionRow = container.querySelector<HTMLElement>(
      '[data-desktop-item="integrated-session"]',
    )!;
    const grip = draftRow.querySelector<HTMLElement>(".cowboy-session-grip")!;
    const from = grip.getBoundingClientRect();
    const to = sessionRow.getBoundingClientRect();
    const pointer = (target: EventTarget, type: string, x: number, y: number) =>
      target.dispatchEvent(
        new PointerEvent(type, {
          bubbles: true,
          pointerId: 1,
          pointerType: "mouse",
          button: 0,
          buttons: type === "pointerup" ? 0 : 1,
          clientX: x,
          clientY: y,
        }),
      );
    pointer(grip, "pointerdown", from.x + 10, from.y + 10);
    await tick();
    pointer(window, "pointermove", from.x + 10, to.y + to.height / 2);
    await tick();
    pointer(window, "pointerup", from.x + 10, to.y + to.height / 2);
    await until(() => copied.length === 1, "Grip drop creates a Session draft");
    check(
      copied[0]!.text === integratedBody &&
        repo.document(id).get().document!.body === integratedBody,
      "Copy keeps exact source and unsent target text",
    );
    await until(
      () =>
        [...document.querySelectorAll("button")].some((button) =>
          button.textContent === "Undo"
        ),
      "Copy snackbar has actual Undo",
    );
    [...document.querySelectorAll<HTMLElement>("[role=alert] button")].find((
      item,
    ) => item.textContent === "Undo")!.click();
    await until(
      () => copied.length === 0,
      "Undo removes the exact copied Session draft",
    );
    // Desktop shows document actions once, in the bottom bar.
    button("Copy to Session").click();
    await until(
      () =>
        !!document.querySelector(
          '[role="tree"][aria-label="Destination Sessions"]',
        ),
      "Production Add to Session opens the tree picker",
    );
    document.querySelector<HTMLElement>(
      '[data-session-destination-session="integrated-session"]',
    )!.click();
    await until(
      () => copied.length === 1,
      "Tree selection copies to the exact unsent Session destination",
    );
    check(
      copied[0]!.text === integratedBody &&
        repo.document(id).get().document!.body === integratedBody,
      "Tree copy retains original document",
    );
    [...document.querySelectorAll<HTMLElement>("[role=alert] button")].find((
      item,
    ) => item.textContent === "Undo")!.click();
    await until(() => copied.length === 0, "Tree copy has exact snackbar Undo");
    // Leader: ␣ opens which-key, ␣␣ labels sessions, a label opens one.
    {
      const press = (target: Element, key: string, code: string) =>
        flushSync(() =>
          target.dispatchEvent(
            new KeyboardEvent("keydown", {
              key,
              code,
              bubbles: true,
              cancelable: true,
            }),
          )
        );
      const row = container.querySelector<HTMLElement>(
        '[data-desktop-item="integrated-session"]',
      )!;
      row.focus();
      press(row, " ", "Space");
      await until(
        () => !!document.querySelector('[data-desktop-leader-menu="root"]'),
        "Space opens the which-key leader panel",
      );
      check(
        document.querySelector('[data-leader-entry=" "]') &&
          document.querySelector('[data-leader-entry="n"]'),
        "which-key lists ␣␣ Switch Session and ␣N New",
      );
      press(row, " ", "Space");
      await until(
        () => !!document.querySelector('[data-desktop-leader-menu="sessions"]'),
        "␣␣ opens the session switcher layer",
      );
      await until(
        () => !!row.querySelector("[data-session-jump-label]"),
        "Session rows show their switcher label",
      );
      const label = row.querySelector<HTMLElement>("[data-session-jump-label]")
        ?.dataset.sessionJumpLabel;
      check(
        label && /^[a-z]$/.test(label) &&
          document.querySelector(`[data-session-jump-entry="${label}"]`),
        "The session row and the switcher show the same letter label",
      );
      press(row, label, `Key${label.toUpperCase()}`);
      await until(
        () => !container.querySelector("[data-workspace-document]"),
        "Pressing the label opens that session",
      );
      check(
        !document.querySelector("[data-desktop-leader-menu]") &&
          !row.querySelector("[data-session-jump-label]"),
        "Labels and which-key disappear after the jump",
      );
      // Top bar actions run from anywhere through the ␣T group; no focus trip.
      await until(
        () =>
          !!document.querySelector("[data-desktop-topbar-action='usage']"),
        "The open session shows its Top bar",
      );
      // tools/cdp-shortcut-audit.ts: hold the integrated Desktop App with an
      // open session and publish every visible control that has no keyboard
      // slot, so missing shortcuts are found from the real DOM.
      if ((globalThis as { __cowboyShortcutAudit?: boolean }).__cowboyShortcutAudit) {
        (globalThis as { __cowboyShortcutAuditResult?: unknown })
          .__cowboyShortcutAuditResult = shortcutAudit(document);
        await new Promise<void>(() => {});
      }
      const unreachable = shortcutAudit(document);
      check(
        unreachable.length === 0,
        `Every visible Desktop control has a keyboard slot: ${unreachable.join("; ")}`,
      );
      check(
        document.querySelector("[data-desktop-topbar-action='usage']")
          ?.textContent?.includes("␣TU"),
        "Top bar actions show one ␣T slot keycap",
      );
      row.focus();
      press(row, " ", "Space");
      press(row, "t", "KeyT");
      await until(
        () => !!document.querySelector('[data-desktop-leader-menu="group:t"]'),
        "␣T opens the Top bar group in which-key",
      );
      check(
        document.querySelector('[data-leader-entry="u"]') &&
          document.querySelector('[data-leader-entry="t"]'),
        "The group lists Usage and Focus Top Bar",
      );
      press(row, "u", "KeyU");
      const usageDialog = () =>
        [...document.querySelectorAll("[role=dialog]")].some((dialog) =>
          dialog.textContent?.includes("Usage and activity")
        );
      await until(
        usageDialog,
        "␣TU opens Usage without focusing the Top bar first",
      );
      press(document.activeElement!, "Escape", "Escape");
      await until(() => !usageDialog(), "Usage closes");
      // `␣SZ` presses the Sessions fold button from any focus.
      {
        const foldAction = () =>
          container.querySelector<HTMLElement>("[data-desktop-region='sessions.list'] ul")
            ?.dataset.desktopFoldAction;
        const before = foldAction();
        check(before, "Sessions offers a fold action");
        check(
          [...container.querySelectorAll("[aria-keyshortcuts]")].some((element) =>
            element.textContent?.includes("␣SZ")
          ),
          "The fold button shows its ␣SZ slot",
        );
        row.focus();
        press(row, " ", "Space");
        press(row, "s", "KeyS");
        await until(
          () => !!document.querySelector('[data-desktop-leader-menu="group:s"]'),
          "␣S opens the Sessions group",
        );
        press(row, "z", "KeyZ");
        await until(() => foldAction() !== before, "␣SZ runs the Sessions fold button");
      }
      container.querySelector<HTMLElement>(`[data-desktop-item="draft:${id}"]`)!
        .click();
      await until(
        () => !!container.querySelector("[data-workspace-document]"),
        "Draft reopens after the leader jump",
      );
      // Draft title: `␣T` goes to it, Enter returns to the body start, and ↑ on
      // the first body line re-enters the title (FOCUS.md "Draft document").
      {
        const titleField = () =>
          container.querySelector<HTMLInputElement>("input[aria-label='Draft title']");
        await until(() => !!titleField(), "Draft title field");
        check(
          container.querySelector("[data-draft-title-shortcut]")?.textContent
            ?.includes("␣T"),
          "The title shows its ␣T slot",
        );
        const leader = isMac ? { metaKey: true } : { altKey: true };
        const at = document.activeElement ?? document.body;
        flushSync(() =>
          at.dispatchEvent(new KeyboardEvent("keydown", { key: "k", code: "KeyK", bubbles: true, cancelable: true, ...leader }))
        );
        press(document.activeElement ?? document.body, "t", "KeyT");
        await tick();
        const field = titleField()!;
        check(
          document.activeElement === field &&
            field.selectionStart === field.value.length &&
            field.selectionEnd === field.value.length,
          "␣T puts the cursor at the end of the title",
        );
        press(field, "Enter", "Enter");
        await tick();
        check(
          document.activeElement !== field &&
            !!document.activeElement?.closest(".cm-editor"),
          "Enter in the title returns to the body",
        );
        press(document.activeElement!, "ArrowUp", "ArrowUp");
        await tick();
        check(
          document.activeElement === field &&
            field.selectionStart === field.value.length,
          "↑ on the first body line enters the title at its end",
        );
        press(field, "Escape", "Escape");
        await tick();
        check(
          !!document.activeElement?.closest(".cm-editor"),
          "Esc in the title returns to the body without closing anything",
        );
      }
      // `␣⇥` flips back to the previous item; `␣O` lists Recent, opened on
      // that item, and a digit opens a row at once.
      {
        const leaderKey = (target: Element) =>
          flushSync(() =>
            target.dispatchEvent(
              new KeyboardEvent("keydown", {
                key: "k",
                code: "KeyK",
                bubbles: true,
                cancelable: true,
                ...(isMac ? { metaKey: true } : { altKey: true }),
              }),
            )
          );
        leaderKey(document.activeElement ?? document.body);
        press(document.activeElement ?? document.body, "Tab", "Tab");
        await until(
          () => !container.querySelector("[data-workspace-document]"),
          "␣⇥ returns to the previous Session",
        );
        leaderKey(document.activeElement ?? document.body);
        press(document.activeElement ?? document.body, "o", "KeyO");
        await until(
          () => !!document.querySelector("[data-desktop-recent]"),
          "␣O opens Recent",
        );
        const first = document.querySelector<HTMLElement>(
          '[data-desktop-recent-index="0"]',
        );
        check(
          first?.dataset.desktopRecentKey === `draft:${id}` &&
            first.getAttribute("aria-selected") === "true",
          "Recent opens on the previous item, the Draft",
        );
        check(
          !document.querySelector(
            '[data-desktop-recent-key="integrated-session"]',
          ),
          "Recent leaves out the current Session",
        );
        await tick(100);
        press(document.activeElement ?? document.body, "1", "Digit1");
        await until(
          () => !!container.querySelector("[data-workspace-document]") &&
            !document.querySelector("[data-desktop-recent]"),
          "1 in Recent reopens the Draft and closes the dialog",
        );
      }
      // `'` labels the focused list's rows; a label moves the cursor.
      const draftRow = container.querySelector<HTMLElement>(
        `[data-desktop-item="draft:${id}"]`,
      )!;
      draftRow.focus();
      await tick();
      press(draftRow, "'", "Quote");
      await until(
        () => document.querySelectorAll("[data-desktop-hint]").length >= 2,
        "' labels every visible Sessions row",
      );
      const hint = [...document.querySelectorAll<HTMLElement>("[data-desktop-hint]")]
        .find((element) => {
          const rect = element.getBoundingClientRect();
          const target = row.getBoundingClientRect();
          return rect.top >= target.top && rect.bottom <= target.bottom;
        })?.dataset.desktopHint;
      check(hint, "The session row carries a label");
      press(draftRow, hint, `Key${hint.toUpperCase()}`);
      await tick();
      check(
        document.activeElement === row &&
          !document.querySelector("[data-desktop-hint]"),
        "A label moves the list cursor and clears every label",
      );
      // Move pick: `m` on a row darkens the page, lights a letter on every
      // folder, and the letter files the row there; Esc leaves untouched.
      {
        const banner = () => document.querySelector("[data-move-pick-banner]");
        const left = () => row.getBoundingClientRect().left;
        const unfiled = left();
        press(row, "m", "KeyM");
        await until(
          () => !!banner() && !!document.querySelector("[data-desktop-move-spotlight]"),
          "m starts Move pick with the page darkened",
        );
        const folderLabel = container.querySelector<HTMLElement>(
          "[data-desktop-folder-row] [data-move-pick-label]",
        )?.dataset.movePickLabel;
        check(
          folderLabel && /^[a-z]$/.test(folderLabel) && banner()?.textContent?.includes("Esc"),
          "Folders carry home-row letters and the banner shows Esc",
        );
        press(document.activeElement ?? document.body, "Escape", "Escape");
        await until(() => !banner(), "Esc leaves Move pick");
        check(left() === unfiled, "Esc moves nothing");
        row.focus();
        press(row, "m", "KeyM");
        await until(() => !!banner(), "m starts Move pick again");
        press(document.activeElement ?? document.body, "q", "KeyQ");
        await tick();
        check(banner(), "A key that names no folder keeps the pick open");
        const label = container.querySelector<HTMLElement>(
          "[data-desktop-folder-row] [data-move-pick-label]",
        )!.dataset.movePickLabel!;
        press(document.activeElement ?? document.body, label, `Key${label.toUpperCase()}`);
        await until(
          () => !banner() && left() > unfiled,
          "The folder's letter files the session into it",
        );
        const moved = container.querySelector<HTMLElement>(
          '[data-desktop-item="integrated-session"]',
        )!;
        moved.focus();
        press(moved, "m", "KeyM");
        await until(() => !!banner(), "Move pick opens for the filed session");
        const top = banner()!.querySelector<HTMLElement>("[data-move-pick-label]")
          ?.dataset.movePickLabel;
        check(top, "Top level carries a letter once the session is filed");
        press(document.activeElement ?? document.body, top, `Key${top.toUpperCase()}`);
        await until(
          () => !banner() &&
            container.querySelector<HTMLElement>('[data-desktop-item="integrated-session"]')!
                .getBoundingClientRect().left === unfiled,
          "Top level's letter returns it",
        );
      }
      // Inside a modal the leader labels that modal's own controls.
      const modifier = isMac ? { metaKey: true } : { altKey: true };
      const prefix = (target: Element) =>
        flushSync(() =>
          target.dispatchEvent(
            new KeyboardEvent("keydown", {
              key: "k",
              code: "KeyK",
              bubbles: true,
              cancelable: true,
              ...modifier,
            }),
          )
        );
      prefix(row);
      press(row, "n", "KeyN");
      await until(
        () => !!document.querySelector('[role="tab"][aria-label="Draft"]'),
        "␣N opens Create",
      );
      await tick(250);
      const title = document.activeElement!;
      check(title.matches("input"), "Create focuses its title");
      prefix(title);
      await until(
        () => !!document.querySelector('[data-desktop-leader-menu="modal"]'),
        "The leader inside a dialog lists that dialog's controls",
      );
      const draftEntry = [
        ...document.querySelectorAll<HTMLElement>("[data-modal-leader-entry]"),
      ].find((entry) => entry.textContent?.endsWith("Draft"));
      check(draftEntry, "The Draft tab has a dialog label");
      const dialogLabel = draftEntry.dataset.modalLeaderEntry!;
      check(
        document.querySelector(`[data-desktop-hint="${dialogLabel}"]`),
        "Every dialog label is also painted on its control",
      );
      press(
        title,
        dialogLabel,
        /^\d$/.test(dialogLabel)
          ? `Digit${dialogLabel}`
          : `Key${dialogLabel.toUpperCase()}`,
      );
      await tick();
      check(
        document.querySelector('[role="tab"][aria-label="Draft"]')
            ?.getAttribute("aria-selected") === "true" &&
          !document.querySelector("[data-desktop-hint]"),
        "A dialog label activates its control and clears the layer",
      );
      press(document.activeElement!, "Escape", "Escape");
      press(document.activeElement!, "Escape", "Escape");
      await until(
        () => !document.querySelector('[role="tab"][aria-label="Draft"]'),
        "Create closes",
      );
    }
    // Installable editor plugin through the production manager: the packed
    // example file, permission review, sandbox, toolbar + palette command,
    // editor undo, settings, plugin isolation and uninstall.
    {
      const pluginResponse = await originalFetch("/editor-plugin.cowboy-plugin");
      check(pluginResponse.ok, "Runner serves the packed example plugin");
      const pluginFile = new File(
        [await pluginResponse.text()],
        "text-tools-1.0.0.cowboy-plugin",
        { type: "application/json" },
      );
      await until(
        () => activeEditorExtensionPort()?.context.kind === "document",
        "Draft editor is bound before installing",
      );
      const content = container.querySelector<HTMLElement>(".cm-content")!;
      const view = EditorView.findFromDOM(content)!;
      view.focus();
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: "pear\napple\nfig" },
        selection: { anchor: 0, head: "pear\napple\nfig".length },
        userEvent: "input.type",
      });
      const port = activeEditorExtensionPort()!;
      openEditorExtensions();
      await until(
        () => [...document.querySelectorAll('[role="tab"]')].some((t) => t.textContent === "Extensions"),
        "Editor extensions opens",
      );
      [...document.querySelectorAll<HTMLElement>('[role="tab"]')]
        .find((t) => t.textContent === "Extensions")!.click();
      await until(() => !!document.querySelector("[data-editor-plugin-file]"), "Plugin manager is shown");
      const input = document.querySelector<HTMLInputElement>("[data-editor-plugin-file]")!;
      const transfer = new DataTransfer();
      transfer.items.add(pluginFile);
      input.files = transfer.files;
      input.dispatchEvent(new Event("change", { bubbles: true }));
      await until(() => !!document.querySelector("[data-editor-plugin-review]"), "Install review lists permissions");
      const review = document.querySelector<HTMLElement>("[data-editor-plugin-review]")!;
      check(
        review.textContent?.includes("Read the text and selection") &&
          review.textContent.includes("Replace the selection"),
        "Review shows the exact requested permissions",
      );
      document.querySelector<HTMLElement>("[data-editor-plugin-confirm]")!.click();
      await until(
        () => document.querySelector('[data-editor-plugin="text-tools"]')
          ?.getAttribute("data-editor-plugin-status") === "running",
        "Installed plugin runs",
      );
      const sandbox = document.querySelector<HTMLIFrameElement>("[data-cowboy-editor-plugin-sandbox]");
      check(
        sandbox?.getAttribute("sandbox") === "allow-scripts",
        "Plugin runs in an opaque-origin sandbox",
      );
      closeEditorExtensions();
      await until(
        () => !!container.querySelector('[data-editor-plugin-command="text-tools:sort-lines"]'),
        "Plugin contributes a Desktop toolbar button",
      );
      // Plugin buttons must not push the toolbar past narrow panes or large fonts.
      const draftToolbar = container.querySelector<HTMLElement>("[data-desktop-draft-toolbar]")!;
      for (const [width, font] of [[1200, 16], [800, 32], [640, 24], [480, 24], [480, 16]] as const) {
        container.style.width = `${width}px`;
        document.documentElement.style.fontSize = `${font}px`;
        await tick();
        check(
          draftToolbar.scrollWidth <= draftToolbar.clientWidth + 1 &&
            [...draftToolbar.querySelectorAll<HTMLElement>("[data-editor-plugin-command]")]
              .every((b) => b.getBoundingClientRect().right <= draftToolbar.getBoundingClientRect().right + 1),
          `Plugin toolbar fits ${width}px at ${font}px: ${draftToolbar.scrollWidth}/${draftToolbar.clientWidth} ${[...draftToolbar.querySelectorAll<HTMLElement>("[data-editor-plugin-command]")].map((b) => Math.round(b.getBoundingClientRect().right)).join(",")} vs ${Math.round(draftToolbar.getBoundingClientRect().right)} ${getComputedStyle(draftToolbar).display} ${[...draftToolbar.children].map((c) => c.tagName + ":" + Math.round(c.getBoundingClientRect().width) + ":" + (c as HTMLElement).scrollWidth).join(" ")}`,
        );
      }
      container.style.width = "1200px";
      document.documentElement.style.fontSize = "16px";
      await tick();
      view.focus();
      view.dispatch({ selection: { anchor: 0, head: view.state.doc.length } });
      container.querySelector<HTMLElement>('[data-editor-plugin-command="text-tools:sort-lines"]')!.click();
      await until(() => port.read().text === "apple\nfig\npear", "Toolbar runs the plugin command");
      check(undo(view), "Plugin edit is one ordinary undo step");
      check(port.read().text === "pear\napple\nfig", "Undo restores the exact pre-plugin text");
      // The palette path runs the same registered command.
      view.dispatch({ selection: { anchor: 0, head: view.state.doc.length } });
      await editorPluginHost().updateSettings("text-tools", { order: "desc" });
      await tick(100);
      await editorPluginHost().runCommand("text-tools", "sort-lines", port);
      check(port.read().text === "pear\nfig\napple", "Settings reach the running plugin");
      // A hostile plugin cannot reach the network, App storage or parent DOM,
      // and its failure leaves the other plugin running.
      const probeManifest = {
        id: "probe",
        name: "Probe",
        version: "1.0.0",
        description: "Sandbox probe",
        author: "Conformance",
        api: { major: 1, minor: 0 },
        permissions: [],
        contexts: ["document"],
        surfaces: ["desktop"],
        settings: [],
      } as const;
      const probeMain = `definePlugin({ async onload(ctx) {
        const results = [];
        // Bypass the prelude's convenience shadowing: the CSP must still block.
        let realFetch;
        for (let o = self; o && !realFetch; o = Object.getPrototypeOf(o)) {
          const d = Object.getOwnPropertyDescriptor(o, "fetch");
          if (d && typeof d.value === "function") realFetch = d.value;
        }
        self.addEventListener("securitypolicyviolation", (e) => { self.__violation = e.effectiveDirective || e.violatedDirective; });
        if (!realFetch) results.push("no-fetch");
        else {
          try { await realFetch.call(self, ${JSON.stringify(`${location.origin}/api/sync/dataset`)}); results.push("fetch-open"); }
          catch { results.push("fetch-blocked"); }
        }
        await new Promise((r) => setTimeout(r, 50));
        results.push(self.origin === "null" ? "opaque-origin" : "app-origin:" + self.origin);
        results.push(typeof document === "undefined" ? "no-dom" : "dom");
        ctx.addCommand({ id: "report", title: results.join(","), description: "csp:" + (self.__violation || "none"), run() {} });
        ctx.addCommand({ id: "explode", title: "Explode", run() { throw new Error("boom"); } });
      }});`;
      const { editorPluginDigest } = await import("./editorPlugins/manifest");
      const probePackage = JSON.stringify({
        format: "cowboy-editor-plugin/1",
        manifest: probeManifest,
        main: probeMain,
        digest: await editorPluginDigest(probeManifest, probeMain),
      });
      const probe = await editorPluginHost().install(await editorPluginHost().inspect(probePackage));
      check(probe.ok, `Probe plugin installs: ${probe.message}`);
      const probeView = editorPluginHost().getSnapshot().plugins.find((p) => p.manifest.id === "probe")!;
      const report = probeView.commands.find((c) => c.id === "report")!.title;
      check(report === "fetch-blocked,opaque-origin,no-dom", `Sandbox isolation: ${report}`);
      const csp = probeView.commands.find((x) => x.id === "report")!.description;
      check(csp === "csp:connect-src", `The sandbox CSP blocks network: ${csp}`);
      (globalThis as { __pluginCsp?: string }).__pluginCsp = csp;
      let exploded = false;
      try {
        await editorPluginHost().runCommand("probe", "explode", port);
      } catch {
        exploded = true;
      }
      check(exploded, "A throwing command reports its failure");
      check(
        editorPluginHost().getSnapshot().plugins.every((p) => p.status === "running"),
        "One command failure does not stop any plugin",
      );
      // Expired authority: the document changes before the plugin writes.
      check(
        !port.replaceSelection("stale", { ...port.read(), revision: port.read().revision - 1 }),
        "A stale document version is refused",
      );
      await editorPluginHost().uninstall("probe");
      await editorPluginHost().uninstall("text-tools");
      await until(
        () => !document.querySelector("[data-cowboy-editor-plugin-sandbox]") &&
          !container.querySelector("[data-editor-plugin-toolbar]"),
        "Uninstall removes sandboxes and toolbar contributions",
      );
      results.push(
        `Installable editor plugin: packed example file installs through the review UI into an opaque-origin sandbox; toolbar and palette run one command with editor undo and live settings; network (${(globalThis as { __pluginCsp?: string }).__pluginCsp}), App storage and DOM are unreachable; failures stay isolated; uninstall removes code and contributions`,
      );
    }
    integrated.unmount();
    touchCreate = true;
    localStorage.setItem("cowboy:mobile-product", "review");
    const mobileRoot = createRoot(container);
    container.style.cssText = "width:390px;height:800px;position:relative";
    flushSync(() =>
      mobileRoot.render(
        <SurfaceProvider>
          <BrowserProductTheme>
            <CssBaseline />
            <AppErrorBoundary>
              <MobileApp themeMode="light" onSetThemeMode={() => {}} />
            </AppErrorBoundary>
          </BrowserProductTheme>
        </SurfaceProvider>,
      )
    );
    await until(
      () => !!container.querySelector("[data-workspace-document]"),
      "Draft in the actual Mobile shell",
    );
    check(
      container.querySelector("[data-mobile-product]")?.getAttribute(
        "data-mobile-product",
      ) === "agent",
      "Restored Review cannot hide a Draft",
    );
    openMobileProduct("review");
    await tick();
    check(
      container.querySelector("[data-mobile-product]")?.getAttribute(
        "data-mobile-product",
      ) === "agent",
      "Draft rejects Code pager navigation",
    );
    check(
      !container.querySelector("[data-mobile-open-code]"),
      "Draft hides Code controls",
    );
    check(
      container.querySelector(
        "[data-mobile-drawer-surface='true'] [data-workspace-document]",
      ),
      "Draft uses the existing Sessions drawer surface",
    );
    {
      // The formatting bar rests at the bottom with the keyboard away; there
      // is no navigation capsule, and Hide keyboard needs an actual keyboard.
      // Focus alone (before the keyboard rises) must not move the bar, or it
      // flashes onto the home indicator as the keyboard opens.
      (document.activeElement as HTMLElement | null)?.blur();
      await tick();
      const bar = container.querySelector<HTMLElement>("[data-draft-mobile-toolbar]");
      check(bar, "Touch Draft renders the formatting capsule");
      const surface = container.querySelector<HTMLElement>("[data-workspace-document]")!;
      const rest = bar.getBoundingClientRect();
      check(
        getComputedStyle(bar).display === "flex" && rest.height > 0 &&
          bar.querySelectorAll("[data-draft-tool]").length > 0 &&
          !container.querySelector("[data-draft-hide-keyboard]"),
        "Formatting bar is visible with the keyboard hidden, without Hide keyboard",
      );
      check(
        !container.querySelector("[data-draft-mobile-nav]") &&
          !container.querySelector('[aria-label="Back"], [aria-label="Forward"]'),
        "No navigation capsule on the touch Draft page",
      );
      check(
        surface.getBoundingClientRect().bottom - rest.bottom < 40,
        `Resting bar sits at the bottom (${Math.round(surface.getBoundingClientRect().bottom - rest.bottom)}px)`,
      );
      container.querySelector<HTMLElement>("[data-draft-body] .cm-content, [data-draft-body] textarea")
        ?.focus();
      await tick();
      const focused = bar.getBoundingClientRect();
      check(
        Math.abs(focused.bottom - rest.bottom) < 1 && Math.abs(focused.width - rest.width) < 1 &&
          !container.querySelector("[data-draft-hide-keyboard]"),
        `Focus before the keyboard rises leaves the bar in place (${Math.round(rest.bottom)} → ${Math.round(focused.bottom)})`,
      );
      document.documentElement.style.setProperty("--kb-inset", "300px");
      await tick();
      const lifted = bar.getBoundingClientRect();
      document.documentElement.style.removeProperty("--kb-inset");
      check(
        lifted.bottom < rest.bottom - 250,
        `A keyboard inset lifts the bar (${Math.round(rest.bottom)} → ${Math.round(lifted.bottom)})`,
      );
    }
    mobileRoot.unmount();
    results.push(
      "Integrated App mixes Draft and Session in the same directory tree; selection retains sidebar DOM, folds and exact shared-editor content without a separate mode; actual Mobile shell keeps Draft on Agent and rejects the Code pager",
    );
    return results;
  } finally {
    try {
      root.unmount();
    } catch { /* already unmounted */ }
    await repo.dispose();
    container.remove();
    document.documentElement.style.fontSize = originalFont;
    globalThis.WebSocket = originalWebSocket;
    device.proof = originalProof;
    globalThis.fetch = originalFetch;
    globalThis.matchMedia = originalMatchMedia;
  }
}

export { mountDraftNativeInputFixture } from "./draftNativeInputFixture";

export function mountIntegratedDraftNativeInputFixture(): void {
  void runDraftDocumentsBrowserConformance(true);
}
