/** Real notice + Undo history over synthetic HTTP: the notice leaves after
 *  UNDO_NOTICE_MS, the history keeps every entry with its state, and a
 *  failed undo explains itself instead of staying offered. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline, Menu, MenuItem } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { DocumentNotifications } from "./documents/DocumentNotifications";
import { UndoHistoryHost } from "./UndoHistorySheet";
import { recordUndo } from "./undoActions";
import { readUndoHistory, UNDO_NOTICE_MS, updateUndoEntry } from "./undoHistory";

export async function runUndoHistoryBrowserChecks(): Promise<void> {
  const check = (ok: unknown, message: string): void => {
    if (!ok) throw new Error(`undo history: ${message}`);
  };
  const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
  const until = async (ready: () => boolean, what: string) => {
    for (let i = 0; i < 100; i++) {
      if (ready()) return;
      await wait(30);
    }
    throw new Error(`undo history: timed out waiting for ${what} [${errors.join(" | ")}] ${document.body.textContent?.slice(0, 200) ?? ""}`);
  };
  const button = (label: string, scope: ParentNode = document) =>
    [...scope.querySelectorAll<HTMLButtonElement>("button")].find((b) =>
      b.textContent?.trim() === label || b.getAttribute("aria-label") === label
    );
  const alert = () => document.querySelector('[role="alert"]');
  const errors: string[] = [];
  const onError = (event: ErrorEvent) => errors.push(String(event.error ?? event.message));
  globalThis.addEventListener("error", onError);
  const realConsoleError = console.error;
  console.error = (...args: unknown[]) => {
    errors.push(args.map(String).join(" ").slice(0, 300));
    realConsoleError(...args);
  };
  const realFetch = globalThis.fetch;
  const restored: string[][] = [];
  let failNext = false;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === "/api/sessions/parent/calls/dismiss") {
      if (failNext) return new Response("gone", { status: 409 });
      const body = JSON.parse(String(init?.body)) as { call_ids: string[] };
      restored.push(body.call_ids);
      return Response.json({ call_ids: body.call_ids });
    }
    return realFetch(input, init);
  }) as typeof fetch;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    flushSync(() =>
      root.render(
        <SurfaceProvider>
          <BrowserProductTheme>
            <CssBaseline />
            <DocumentNotifications />
            <UndoHistoryHost />
          </BrowserProductTheme>
        </SurfaceProvider>,
      )
    );
    recordUndo("2 calls cleared", [{ kind: "restore_calls", parent: "parent", ids: ["a", "b"] }]);
    await until(() => alert()?.textContent?.includes("2 calls cleared") === true, "the notice");
    check(button("Undo", alert()!) && button("Undo history", alert()!), "the notice offers Undo and history");
    await wait(UNDO_NOTICE_MS - 600);
    check(alert(), "the notice is still visible before its duration");
    await until(() => alert() === null, "the notice to leave after its duration");

    recordUndo("1 call cleared", [{ kind: "restore_calls", parent: "parent", ids: ["c"] }]);
    await until(() => Boolean(alert() && button("Undo history", alert()!)), "the second notice");
    // A menu opened while the notice is up must cover it, not the reverse.
    const menuHost = document.createElement("div");
    document.body.append(menuHost);
    const menuRoot = createRoot(menuHost);
    try {
      flushSync(() =>
        menuRoot.render(
          <BrowserProductTheme>
            <Menu
              open
              anchorReference="anchorPosition"
              anchorPosition={{ top: globalThis.innerHeight, left: 0 }}
              slotProps={{ paper: { "data-overlap-menu": "" } as object }}
            >
              {Array.from({ length: 12 }, (_, i) => <MenuItem key={i}>Item {i}</MenuItem>)}
            </Menu>
          </BrowserProductTheme>,
        )
      );
      await until(() => document.querySelector("[data-overlap-menu]") !== null, "the overlapping menu");
      const box = alert()!.getBoundingClientRect();
      const top = document.elementFromPoint(box.left + 12, box.top + box.height / 2);
      check(top?.closest("[data-overlap-menu]"), "an open menu stacks above the notice");
    } finally {
      flushSync(() => menuRoot.unmount());
      menuHost.remove();
    }
    flushSync(() => button("Undo history", alert()!)!.click());
    await until(() => document.querySelector("[data-undo-history]") !== null, "the history sheet");
    const rows = () => [...document.querySelectorAll<HTMLElement>("[data-undo-entry]")];
    check(rows().length === 2, "both changes are listed");
    check(rows()[0]!.textContent?.includes("1 call cleared"), "newest first");
    check(
      rows().every((row) => row.querySelector('[data-undo-state="available"]')),
      "both start as Can undo",
    );

    flushSync(() => button("Undo", rows()[0]!)!.click());
    await until(() => rows()[0]!.querySelector('[data-undo-state="used"]') !== null, "Undone");
    check(JSON.stringify(restored) === JSON.stringify([["c"]]), "undo restores exactly its own calls");
    check(!button("Undo", rows()[0]!), "an undone entry offers no action");

    failNext = true;
    flushSync(() => button("Undo", rows()[1]!)!.click());
    await until(() => rows()[1]!.querySelector('[data-undo-state="unavailable"]') !== null, "Unavailable");
    check(rows()[1]!.textContent?.includes("retry safely"), "a failed undo shows its reason");
    check(!button("Undo", rows()[1]!), "a failed undo is not offered again");
    check(
      globalThis.localStorage.getItem("cowboy.undoHistory.v1")?.includes("1 call cleared"),
      "history survives a reload",
    );
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    globalThis.fetch = realFetch;
    globalThis.localStorage.removeItem("cowboy.undoHistory.v1");
    globalThis.removeEventListener("error", onError);
    console.error = realConsoleError;
  }
}

/** Visual preview for screenshots: one entry in each settled state. */
export function renderUndoHistoryPreview(): void {
  globalThis.fetch = (async () => Response.json({ call_ids: [] })) as typeof fetch;
  const container = document.createElement("div");
  document.body.append(container);
  createRoot(container).render(
    <SurfaceProvider>
      <BrowserProductTheme>
        <CssBaseline />
        <DocumentNotifications />
        <UndoHistoryHost />
      </BrowserProductTheme>
    </SurfaceProvider>,
  );
  recordUndo("\"CS-FWK-001 review\" moved to Trash", [{ kind: "restore_calls", parent: "p", ids: ["a"] }]);
  recordUndo("Moved to Drafts: Investigate Marketplace sync retries", [{ kind: "restore_calls", parent: "p", ids: ["b"] }]);
  recordUndo("3 calls cleared", [{ kind: "restore_calls", parent: "p", ids: ["c"] }]);
  const [unavailable, used] = readUndoHistory().slice(1);
  updateUndoEntry(used!.id, { state: "used" });
  updateUndoEntry(unavailable!.id, {
    state: "unavailable",
    reason: "This draft was edited, scheduled, sent or removed; it has been kept",
  });
}
