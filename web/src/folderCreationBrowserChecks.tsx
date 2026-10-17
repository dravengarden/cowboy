import { useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline } from "@mui/material";
import { CreateDialog } from "./App";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import {
  createSessionFolder,
  removeSessionFolder,
  setSessionFolderDefault,
  useStoreSelector,
} from "./store";
import {
  EMPTY_SESSION_FOLDERS,
  type SessionFoldersValue,
} from "./sessionFolders";
import { managementEntryFixture } from "./providerManagement.fixture";
import { resetProviderCatalog } from "./providerCatalogRegistry";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 100));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
const button = (text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")]
    .find((b) => b.textContent?.trim().startsWith(text));
async function click(element: HTMLElement | null | undefined) {
  check(element, "expected creation control");
  flushSync(() => element.click());
  await tick();
}
function inputValue(input: HTMLInputElement, value: string) {
  flushSync(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!
      .call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

/** Actual Create dialog and optimistic domain, with isolated data and HTTP. */
export function renderFolderCreationPreview(
  mobile = false,
  folderOnly = false,
) {
  const originalFetch = globalThis.fetch;
  const entries = ["claude-code", "codex"].map((id) => {
    const entry = managementEntryFixture(id);
    entry.manifest.display.name = id === "codex" ? "Codex" : "Claude";
    entry.publisher = entry.manifest.publisher;
    if (id === "claude-code") {
      entry.manifest.configuration.presets[0]!.id = "opus-default";
      entry.manifest.configuration.presets[0]!.name = "Opus · Recommended";
    }
    return entry;
  });
  globalThis.fetch = ((input: RequestInfo | URL) => {
    const url = String(input);
    if (url === "/api/plugins") {
      return Promise.resolve(
        Response.json({
          providers: entries,
          authentications: [],
          authentication_executors: [],
        }),
      );
    }
    if (url === "/api/sync/dataset") {
      return Promise.resolve(Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"d".repeat(64)}`,
        user_id: "defaults-fixture",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      }));
    }
    return Promise.resolve(Response.json({}));
  }) as typeof fetch;
  resetProviderCatalog();
  bindProductSyncPrincipal("defaults-fixture");
  const parent = createSessionFolder("Suger", null, null, {
    project: "suger",
    machine: "hawk",
    provider: "claude-code",
    preset: "claude-code/opus-default",
  })!;
  const other = createSessionFolder("Other projects", null, null, {
    project: "cowboy",
    machine: "falcon",
    provider: "claude-code",
  })!;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const profile = classifySurface({
    finePointer: !mobile,
    hover: !mobile,
    tabletWidth: false,
    touchCapable: mobile,
    desktopHost: !mobile,
  });
  const created: string[] = [];
  let current: SessionFoldersValue = EMPTY_SESSION_FOLDERS;
  let reopen = () => {};
  function Fixture() {
    current = useStoreSelector((state) => state.sessionFolders);
    const [open, setOpen] = useState(true);
    reopen = () => setOpen(true);
    return (
      <SurfaceContext.Provider value={profile}>
        <BrowserProductTheme>
          <CssBaseline />
          <CreateDialog
            open={open}
            initialVariant="folder"
            folderOnly={folderOnly}
            initialFolder={parent}
            onClose={() => setOpen(false)}
            onCreated={() => {
              throw new Error("Folder must not create a Session");
            }}
            onFolderCreated={(id) => created.push(id)}
          />
        </BrowserProductTheme>
      </SurfaceContext.Provider>
    );
  }
  flushSync(() => root.render(<Fixture />));
  return {
    created,
    parent,
    other,
    snapshot: () => current,
    reopen: () => flushSync(reopen),
    updateParent: () =>
      flushSync(() => setSessionFolderDefault(parent, "machine", "falcon")),
    close: () => {
      flushSync(() => root.unmount());
      container.remove();
      for (const id of [...created, parent, other]) removeSessionFolder(id);
      globalThis.fetch = originalFetch;
      resetProviderCatalog();
    },
  };
}

export async function runFolderCreationBrowserChecks(): Promise<void> {
  for (const mobile of [false, true]) {
    const fixture = renderFolderCreationPreview(mobile, mobile);
    const name = () =>
      document.querySelector<HTMLInputElement>(
        'input[name="cowboy-session-folder"]',
      )!;
    const toggle = () =>
      document.querySelector<HTMLElement>("[data-folder-defaults-toggle]");
    const field = (key: string) =>
      document.querySelector<HTMLElement>(`[data-default-field="${key}"]`);
    try {
      await tick();
      await tick();
      const count = fixture.snapshot().folders.length;
      check(
        toggle()?.getAttribute("aria-expanded") === "false",
        "defaults start discoverable and collapsed",
      );
      check(
        toggle()?.textContent?.includes("Inherits from Suger"),
        "collapsed summary shows parent inheritance",
      );
      if (mobile) {
        check(
          !document.querySelector('[role="tablist"]'),
          "directory shortcut stays on folder creation",
        );
      }
      inputValue(name(), "Stormbird");
      await click(toggle());
      await click(field("provider"));
      check(
        document.querySelectorAll('[role="dialog"]').length === 1,
        "create defaults reuse one dialog",
      );
      await click(document.querySelector('[data-default-value="codex"]'));
      check(
        name().value === "Stormbird",
        "returning from a picker keeps the authored name",
      );
      check(
        button("Create folder")?.disabled,
        "incompatible inherited preset cannot be silently saved",
      );
      await click(field("preset"));
      await click(document.querySelector('[data-default-mode="clear"]'));
      check(
        !button("Create folder")?.disabled,
        "agent default resolves the incompatible preset",
      );
      fixture.updateParent();
      await tick();
      check(
        field("machine")?.textContent?.includes("falcon"),
        "parent changes update untouched fields live",
      );
      check(
        field("provider")?.textContent?.includes("Codex"),
        "parent changes preserve explicit selections",
      );
      await click(document.querySelector('[role="combobox"]'));
      await click(
        [...document.querySelectorAll<HTMLElement>(
          '#workspace-picker-menu [role="menuitem"]',
        )]
          .find((item) => item.textContent?.includes("Other projects")),
      );
      check(
        field("project")?.textContent?.includes("cowboy"),
        "choosing another parent recomputes inherited project",
      );
      check(
        field("provider")?.textContent?.includes("Codex"),
        "changing parent retains explicit defaults",
      );
      if (!mobile) {
        await click(document.querySelector('[role="tab"][aria-label="Draft"]'));
        await click(
          document.querySelector('[role="tab"][aria-label="Folder"]'),
        );
        check(
          name().value === "Stormbird" &&
            field("provider")?.textContent?.includes("Codex"),
          "switching create tabs preserves the folder draft",
        );
      }
      check(
        fixture.snapshot().folders.length === count,
        "editing defaults does not create a partial folder",
      );
      await click(button("Cancel"));
      check(
        Number(fixture.created.length) === 0,
        "cancel never commits a folder",
      );
      fixture.reopen();
      await tick();
      check(
        name().value === "" &&
          toggle()?.getAttribute("aria-expanded") === "false",
        "reopening starts a fresh draft",
      );
      inputValue(name(), "Review");
      await click(toggle());
      await click(field("project"));
      await click(document.querySelector('[data-default-mode="clear"]'));
      const confirm = button("Create folder");
      check(confirm && !confirm.disabled, "folder is ready to create");
      flushSync(() => {
        confirm.click();
        confirm.click();
      });
      await tick();
      check(fixture.created.length === 1, "repeated submit creates one folder");
      const saved = fixture.snapshot().folders.find((f) =>
        f.id === fixture.created[0]
      );
      check(
        saved?.name === "Review" && saved.parent === fixture.parent,
        "creation saves name and parent together",
      );
      check(
        JSON.stringify(saved.defaults) === JSON.stringify({ project: null }),
        "only explicit overrides are saved on creation",
      );
    } finally {
      fixture.close();
    }
  }
}
