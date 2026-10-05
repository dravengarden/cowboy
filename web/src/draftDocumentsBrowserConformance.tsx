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
import { activeEditorExtensionPort } from "./editorExtensions/host";
import { EditorExtensionsDialog } from "./editorExtensions/EditorExtensionsDialog";
import { DocumentNotifications } from "./documents/DocumentNotifications";

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
              </BrowserProductTheme>
            </SurfaceProvider>
          </StrictMode>,
        )
      );
      await tick(180);
      const tab = (name: string) => {
        const item = [...document.querySelectorAll<HTMLElement>("[role=tab]")]
          .find((element) => element.textContent === name);
        check(item, `Create variant ${name}`);
        item.focus();
        return item;
      };
      check(
        button("Create session").hasAttribute("disabled"),
        "No Machine disables Session only",
      );
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
      body: "Remote writer",
      revision: before.revision + 1,
      body_revision: before.body_revision + 1,
    });
    const localBeforeRefresh = port.read().text;
    await repo.document(id).refresh();
    await tick();
    check(
      port.read().text === localBeforeRefresh,
      "Remote refresh does not replace the live IME/undo document",
    );
    check(
      port.replaceSelection("My competing edit\n", port.read()),
      "Local edit after remote refresh",
    );
    await until(
      () => repo.document(id).get().phase === "conflict",
      "conflict preserved",
    );
    check(
      server.get(id)?.body === "Remote writer",
      "Does not overwrite remote",
    );
    button("Keep mine as copy").click();
    await until(
      () => activeEditorExtensionPort()?.context.id !== id,
      "recovery document opened",
    );
    check(
      activeEditorExtensionPort()?.read().text.includes("My competing edit"),
      "Recovery includes local text",
    );
    results.push(
      "Concurrent writer conflict preserves both documents and opens the recovered local copy",
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

    for (const font of [8, 10.4, 16, 24]) {
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
            `button[aria-label="${label}"]`,
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
      "Actual Desktop workspace, tabs and toolbar fit 320–1440px at 8–24px fonts; modified browser shortcuts are preserved",
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
                    <App
                      themeMode="light"
                      onSetThemeMode={() => {}}
                      surface="desktop"
                    />
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
    button("Copy to Session drafts").click();
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
