import { useLayoutEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { Box, CssBaseline } from "@mui/material";
import { MobileSheetActionGroup } from "@cowboy/app-shell";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { useSessionFoldControl } from "./SessionFoldControl";
import { CreateDialog } from "./App";
import {
  SESSION_FOLDER_CREATED_EVENT,
  type SessionFolderCreated,
} from "./SessionFolderUi";
import { sessionDrawerTargetScroll } from "./mobileDrawerMotion";
import type { SessionFoldersValue } from "./sessionFolders";
import { buildSessionTree, sessionTreeRowKey } from "./sessionTree";
import type { SessionMeta } from "./protocol";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 60));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

const INSET = 84;

/** Real fold hook, tree projection and action island; isolated data. */
export async function runSessionFoldBrowserConformance(): Promise<string[]> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const results: string[] = [];
  const placement: Record<string, string> = {};
  const nested: SessionFoldersValue = {
    folders: [
      ...Array.from({ length: 30 }, (_, i) => ({
        id: `f${i}`,
        name: `Folder ${i}`,
        parent: null,
        project: null,
        position: i,
      })),
      { id: "inner", name: "Inner", parent: "f20", project: null, position: 0 },
    ],
    placement,
  };
  const sessions: SessionMeta[] = [];
  for (let i = 0; i < 30; i++) {
    for (let j = 0; j < 3; j++) {
      const id = `s${i}-${j}`;
      sessions.push({
        id,
        title: id,
        provider: "codex",
        cwd: "/tmp/fixture",
        status: "running",
      });
      placement[id] = i === 20 && j === 1 ? "inner" : `f${i}`;
    }
  }
  let collapsedNow: ReadonlySet<string> = new Set();
  const collapsedCount = () => collapsedNow.size;
  function Fixture(
    { folders, activeId }: {
      folders: SessionFoldersValue;
      activeId: string;
    },
  ) {
    const listRef = useRef<HTMLDivElement>(null);
    const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
    collapsedNow = collapsed;
    const [moved, setMoved] = useState<string | null>(null);
    const tree = buildSessionTree(sessions, folders, collapsed);
    // Mirrors SessionList's moved-row scroll.
    useLayoutEffect(() => {
      const list = listRef.current;
      const item = moved
        ? list?.querySelector<HTMLElement>(`[data-desktop-item="${moved}"]`)
        : null;
      if (!list || !item) return;
      list.scrollTop = sessionDrawerTargetScroll({
        currentScroll: list.scrollTop,
        viewportHeight: list.clientHeight,
        contentHeight: list.scrollHeight,
        itemTop: item.getBoundingClientRect().top -
          list.getBoundingClientRect().top + list.scrollTop,
        itemHeight: item.offsetHeight,
      });
      setMoved(null);
    }, [moved, tree]);
    const fold = useSessionFoldControl({
      listRef,
      tree,
      folders,
      collapsed,
      setCollapsed,
      activeId,
      bottomInset: INSET,
      onLocate: setMoved,
    });
    return (
      <Box sx={{ position: "relative", width: 360, height: 640 }}>
        <Box
          ref={listRef}
          data-fold-list
          sx={{ height: "100%", overflow: "auto", pb: `${INSET}px` }}
        >
          {tree.rows.map((row) => {
            const key = sessionTreeRowKey(row);
            return (
              <Box
                key={key}
                data-desktop-item={row.kind === "empty" ? undefined : key}
                sx={{
                  height: 48,
                  pl: row.depth * 2,
                  display: "flex",
                  alignItems: "center",
                  ...(row.kind === "session" ? fold.pulseSx(key) : {}),
                }}
              >
                {key}
              </Box>
            );
          })}
        </Box>
        <Box sx={{ position: "absolute", left: 16, bottom: 16 }}>
          <MobileSheetActionGroup
            actions={[{
              key: "fold",
              label: fold.label,
              visible: fold.action !== null,
              onPress: fold.run,
              icon: fold.icon,
            }]}
          />
        </Box>
      </Box>
    );
  }
  const render = (folders: SessionFoldersValue, activeId: string) =>
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <SurfaceProvider>
            <Fixture folders={folders} activeId={activeId} />
          </SurfaceProvider>
        </BrowserProductTheme>,
      )
    );
  const list = () => document.querySelector<HTMLElement>("[data-fold-list]")!;
  const button = () => {
    const found = document.querySelector<HTMLElement>(
      "[data-mobile-sheet-footer-shield] button",
    );
    check(found, "missing fold button");
    return found;
  };
  const icon = () =>
    document.querySelector<HTMLElement>("[data-session-fold-icon]")?.dataset
      .sessionFoldIcon;
  const rowTop = (key: string) => {
    const row = document.querySelector<HTMLElement>(
      `[data-desktop-item="${key}"]`,
    );
    check(row, `missing row ${key}`);
    return row.getBoundingClientRect().top - list().getBoundingClientRect().top;
  };
  const inView = (key: string) => {
    const top = rowTop(key);
    return top >= 0 && top + 48 <= list().clientHeight - INSET;
  };
  const press = async () => {
    flushSync(() => button().click());
    await tick();
    await tick();
  };
  try {
    render(nested, "s20-1");
    await tick();
    check(!inView("s20-1"), "fixture current row starts off screen");
    check(
      button().getAttribute("aria-label") === "Focus current session" &&
        icon() === "focus",
      `initial action ${button().getAttribute("aria-label")}`,
    );
    results.push("an off-screen current session arms Focus");

    await press();
    check(
      collapsedNow.size === 29 && !collapsedNow.has("f20") &&
        !collapsedNow.has("inner"),
      `focus collapsed ${[...collapsedNow].join(",")}`,
    );
    check(inView("s20-1"), "focus left the current row off screen");
    const pulsed = document.querySelector<HTMLElement>(
      '[data-desktop-item="s20-1"]',
    )!;
    check(
      getComputedStyle(pulsed).animationName.startsWith("session-locate-"),
      "focus did not pulse the current row",
    );
    check(
      button().getAttribute("aria-label") === "Expand all folders" &&
        icon() === "expand",
      `focused action ${button().getAttribute("aria-label")}`,
    );
    results.push("Focus folds off-path folders, scrolls and rings the row");

    list().scrollTop = list().scrollHeight;
    await tick();
    check(!inView("s20-1"), "the focused list could not scroll away");
    check(
      button().getAttribute("aria-label") === "Focus current session",
      "scrolling away did not re-arm Focus",
    );
    await press();
    check(inView("s20-1"), "a second Focus did not bring the row back");
    check(
      button().getAttribute("aria-label") === "Expand all folders",
      "a returned row did not arm Expand",
    );
    results.push("scrolling away re-arms Focus without unfolding");

    const before = rowTop("s20-1");
    await press();
    check(collapsedCount() === 0, "expand left folders collapsed");
    check(
      Math.abs(rowTop("s20-1") - before) <= 1,
      `expand moved the current row ${before} -> ${rowTop("s20-1")}`,
    );
    check(
      button().getAttribute("aria-label") === "Focus current session",
      "expanded list did not re-arm Focus",
    );
    results.push("Expand all keeps the current row anchored");

    render({ folders: [], placement: {} }, "s3-0");
    await tick();
    check(
      button().getAttribute("aria-label") === "Show current session" &&
        icon() === "locate",
      `flat action ${button().getAttribute("aria-label")}`,
    );
    list().scrollTop = list().scrollHeight;
    await press();
    check(inView("s3-0"), "locate did not bring the row into view");
    results.push("a flat list only locates");

    // "+" creates folders too: Create's Folder tab names and places one.
    const created: SessionFolderCreated[] = [];
    const onCreated = (event: Event) =>
      created.push((event as CustomEvent<SessionFolderCreated>).detail);
    globalThis.addEventListener(SESSION_FOLDER_CREATED_EVENT, onCreated);
    let closed = false;
    try {
      flushSync(() =>
        root.render(
          <SurfaceProvider>
            <BrowserProductTheme>
              <CssBaseline />
              <CreateDialog
                open
                onClose={() => {
                  closed = true;
                }}
                onCreated={() => {
                  throw new Error("Folder must not create a Session");
                }}
              />
            </BrowserProductTheme>
          </SurfaceProvider>,
        )
      );
      await tick();
      await tick();
      const tab = [...document.querySelectorAll<HTMLElement>("[role=tab]")]
        .find((element) => element.textContent === "Folder");
      check(tab, "Create has no Folder tab");
      flushSync(() => tab.click());
      await tick();
      const name = document.querySelector<HTMLInputElement>(
        'input[name="cowboy-session-folder"]',
      );
      check(name, "Folder tab has no name field");
      check(document.activeElement === name, "Folder tab did not focus Name");
      check(
        name.autocomplete === "off" && name.value === "",
        "Folder name must start empty without contact AutoFill",
      );
      const confirm = () =>
        [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) =>
          b.textContent?.startsWith("Create folder")
        );
      check(confirm()?.disabled, "an empty folder name can be created");
      check(
        document.body.textContent?.includes("Inside folder (optional)") &&
          !document.body.textContent.includes("AI installation"),
        "Folder tab shows session-only fields",
      );
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!;
      setter.call(name, "  Research  ");
      name.dispatchEvent(new Event("input", { bubbles: true }));
      await tick();
      check(!confirm()?.disabled, "a named folder cannot be created");
      flushSync(() => confirm()!.click());
      await tick();
      check(closed, "Create stayed open after making a folder");
      check(
        created.length === 1 && created[0]!.id.startsWith("f-") &&
          created[0]!.parent === null,
        `folder announcement ${JSON.stringify(created)}`,
      );
    } finally {
      globalThis.removeEventListener(SESSION_FOLDER_CREATED_EVENT, onCreated);
    }
    results.push("Create's Folder tab names, places and announces a folder");
  } finally {
    root.unmount();
    container.remove();
  }
  return results;
}
