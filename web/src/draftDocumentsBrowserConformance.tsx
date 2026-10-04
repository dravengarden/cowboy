import { StrictMode } from "react";
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
  throw new Error(`Timed out: ${label}`);
}
function button(label: string): HTMLElement {
  const element = [
    ...document.querySelectorAll<HTMLElement>("button,[role=button]"),
  ].find((item) =>
    item.getClientRects().length &&
    (item.getAttribute("aria-label") === label ||
      item.textContent?.trim() === label)
  );
  check(element, `Missing button: ${label}`);
  return element;
}

/** Real Draft workspace, shared editor/extension host and dataset-owned IndexedDB.
 * Only remote HTTP is a fixture. Native keyboard acceptance is a separate run. */
export async function runDraftDocumentsBrowserConformance(): Promise<string[]> {
  const originalFetch = globalThis.fetch;
  const originalMatchMedia = globalThis.matchMedia;
  // Headless Firefox defaults to pointer:none. Exercise the actual Desktop
  // product branch while keeping the real viewport/theme media queries.
  globalThis.matchMedia = (query) => {
    const pointer = query.includes("(pointer: fine)") ||
      query.includes("(hover: hover)");
    const coarse = query.includes("(any-pointer: coarse)");
    if (!pointer && !coarse) return originalMatchMedia.call(globalThis, query);
    return {
      matches: pointer,
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
    await repo.start();
    const folder = await repo.create("Research", null, "folder");
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
        const editor = container.querySelector<HTMLElement>(".cm-editor")!;
        check(
          editor.getBoundingClientRect().height > 200,
          "Editing canvas retains useful height",
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
    return results;
  } finally {
    try {
      root.unmount();
    } catch { /* already unmounted */ }
    await repo.dispose();
    container.remove();
    document.documentElement.style.fontSize = originalFont;
    globalThis.fetch = originalFetch;
    globalThis.matchMedia = originalMatchMedia;
  }
}

export { mountDraftNativeInputFixture } from "./draftNativeInputFixture";
