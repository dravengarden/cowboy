import { useRef, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { Box, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import {
  FolderPickerShell,
  SESSION_FOLDER_SHEET_HOST,
} from "./SessionFolderUi";
import { bindMobileSpatialDrawer } from "./mobileSpatialDrawer";
import { MOBILE_SESSION_DRAWER_WIDTH } from "./mobileDrawerMotion";
import { mobileDrawerRailHitSx } from "./mobilePresentationMotion";
import {
  sessionFolderMutators,
  type SessionFoldersValue,
} from "./sessionFolders";
import {
  buildSessionTree,
  projectSessionDrop,
  sessionTreeRowKey,
} from "./sessionTree";
import { useSortable } from "./useSortable";
import type { SessionMeta } from "./protocol";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 35));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

/** Real picker, sortable/tree projection and drawer controller; isolated data. */
export async function runSessionMoveBrowserConformance(): Promise<string[]> {
  const preview = new URL(location.href).searchParams.has("preview");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const results: string[] = [];
  const initial: SessionFoldersValue = {
    folders: [
      {
        id: "parent",
        name: "Work",
        parent: null,
        project: "cowboy",
        position: 0,
      },
      {
        id: "nested",
        name: "Nested",
        parent: "parent",
        project: null,
        position: 0,
      },
      ...Array.from({ length: 24 }, (_, i) => ({
        id: `folder-${i}`,
        name: `Folder ${i}`,
        parent: null,
        project: null,
        position: i + 1,
      })),
    ],
    placement: { a: "parent", b: "parent", c: "parent" },
  };
  const sessions: SessionMeta[] = ["a", "b", "c"].map((id) => ({
    id,
    title: id,
    provider: "codex",
    cwd: "/tmp/fixture",
    status: "running",
    workspace_name: "cowboy",
  }));
  const picks: (string | null)[] = [];
  let dropCount = 0;
  let latest = initial;
  const render = (children: React.ReactNode) =>
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <SurfaceProvider>{children}</SurfaceProvider>
        </BrowserProductTheme>,
      )
    );
  const picker = (current: string | null, exclude: string | null = null) =>
    render(
      <>
        <div {...{ [SESSION_FOLDER_SHEET_HOST]: "" }} />
        <FolderPickerShell
          key={`${current}:${exclude}`}
          title="Move session to"
          value={initial}
          current={current}
          exclude={exclude}
          onPick={(id) => picks.push(id)}
          onClose={() => {}}
        />
      </>,
    );
  const row = (id: string) => {
    const found = document.querySelector<HTMLElement>(
      `[data-folder-pick="${id}"]`,
    );
    check(found, `Missing picker row ${id}`);
    return found;
  };
  function DragTree() {
    const [value, setValue] = useState<SessionFoldersValue>(initial);
    latest = value;
    const listRef = useRef<HTMLDivElement>(null);
    const tree = buildSessionTree(
      sessions,
      value,
      new Set(initial.folders.map((f) => f.id).filter((id) => id !== "parent")),
    );
    const keys = tree.rows.map(sessionTreeRowKey);
    const byKey = new Map(tree.rows.map((r) => [sessionTreeRowKey(r), r]));
    const sortable = useSortable({
      ids: keys,
      onReorder() {},
      onDrop(_order, drag) {
        const source = byKey.get(drag.id);
        if (source?.kind !== "session") return;
        const others = tree.rows.filter((r) =>
          sessionTreeRowKey(r) !== drag.id
        );
        const target = projectSessionDrop(
          others,
          drag.targetIndex,
          source.depth,
          drag.depthSteps,
        );
        dropCount++;
        setValue((v) =>
          sessionFolderMutators.place(v, {
            session_ids: [drag.id],
            folder: target.folder,
          })
        );
      },
      horizontalStep: 28,
      optimisticReorder: false,
      scrollContainer: () => listRef.current,
    });
    return (
      <Box ref={listRef} sx={{ width: 349, height: 500, overflow: "auto" }}>
        {sortable.order.map((key) => {
          const item = byKey.get(key)!;
          return (
            <Box
              key={key}
              ref={sortable.registerItem(key)}
              style={sortable.itemStyle(key)}
              data-move-row={key}
              sx={{
                height: 48,
                pl: item.depth * 2,
                display: "flex",
                alignItems: "center",
              }}
            >
              {item.kind === "session" && (
                <button data-move-grip={key} {...sortable.handleProps(key)}>
                  Drag
                </button>
              )}
              {key}
            </Box>
          );
        })}
      </Box>
    );
  }
  const pointer = (target: EventTarget, type: string, x: number, y: number) => {
    target.dispatchEvent(
      new PointerEvent(type, {
        bubbles: true,
        button: 0,
        pointerId: 1,
        pointerType: "touch",
        clientX: x,
        clientY: y,
      }),
    );
  };
  try {
    picker("parent");
    await tick();
    check(
      row("parent").getAttribute("aria-disabled") === "true",
      "current folder remains actionable",
    );
    flushSync(() => row("parent").click());
    check(picks.length === 0, "current destination submitted a move");
    const global = row("");
    const list = global.parentElement!;
    list.scrollTop = list.scrollHeight;
    check(
      Math.abs(
        global.getBoundingClientRect().top - list.getBoundingClientRect().top,
      ) < 10,
      "Global scrolled away in the long destination list",
    );
    flushSync(() => global.click());
    check(
      Number(picks.length) === 1 && picks[0] === null,
      "Global did not submit explicit top-level placement",
    );
    results.push(
      "Long destination lists keep Global visible; current location cannot accidentally reorder itself",
    );

    const search = document.querySelector<HTMLInputElement>("input");
    check(search, "Long folder tree has no search");
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    flushSync(() => {
      setter.call(search, "does not exist");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    check(
      document.querySelectorAll("[data-folder-pick]").length === 1,
      "Search did not filter folder results",
    );
    check(
      row("").textContent?.includes("Outside all folders"),
      "Search hid the Global destination",
    );
    picker(null, "parent");
    await tick();
    check(
      row("").getAttribute("aria-disabled") === "true",
      "Already-global item offers an active move to itself",
    );
    check(
      !document.querySelector(
        '[data-folder-pick="parent"], [data-folder-pick="nested"]',
      ),
      "A folder can move into its own subtree",
    );
    results.push(
      "Search retains Global, and folder moves exclude themselves and their descendants",
    );

    render(<DragTree />);
    await tick();
    const grip = document.querySelector<HTMLElement>('[data-move-grip="b"]')!;
    const rect = grip.getBoundingClientRect();
    const x = rect.left + rect.width / 2;
    const y = rect.top + rect.height / 2;
    flushSync(() => pointer(grip, "pointerdown", x, y));
    await tick();
    // Same event turn: no React commit between final horizontal intent and release.
    pointer(window, "pointermove", x - 30, y);
    pointer(window, "pointerup", x - 30, y);
    await tick();
    check(
      latest.placement.b === "" && dropCount === 1,
      "Fast left release did not escape to Global",
    );
    const canonical = buildSessionTree(
      sessions,
      latest,
      new Set(initial.folders.map((f) => f.id).filter((id) => id !== "parent")),
    ).rows.map(sessionTreeRowKey);
    const actual = [
      ...document.querySelectorAll<HTMLElement>("[data-move-row]"),
    ].map((el) => el.dataset.moveRow);
    check(
      JSON.stringify(canonical) === JSON.stringify(actual),
      "Flat drag order overrode the regrouped tree",
    );
    const nextGrip = document.querySelector<HTMLElement>(
      '[data-move-grip="a"]',
    )!;
    const nextRect = nextGrip.getBoundingClientRect();
    flushSync(() =>
      pointer(nextGrip, "pointerdown", nextRect.x + 10, nextRect.y + 10)
    );
    await tick();
    pointer(window, "pointermove", nextRect.x - 35, nextRect.y + 10);
    pointer(window, "pointercancel", nextRect.x - 35, nextRect.y + 10);
    await tick();
    check(
      latest.placement.a === "parent" && dropCount === 1,
      "Cancelled drag committed a move",
    );
    const rootGrip = document.querySelector<HTMLElement>(
      '[data-move-grip="b"]',
    )!;
    rootGrip.scrollIntoView({ block: "nearest" });
    const rootRect = rootGrip.getBoundingClientRect();
    flushSync(() =>
      pointer(rootGrip, "pointerdown", rootRect.x + 10, rootRect.y + 10)
    );
    await tick();
    pointer(window, "pointerup", rootRect.x + 10, rootRect.y + 10);
    await tick();
    check(
      latest.placement.b === "" && dropCount === 1,
      "Tapping a Global row's grip filed it into the preceding collapsed folder",
    );
    results.push(
      "Fast horizontal drop escapes mid-folder to Global; canonical tree order wins; cancellation and grip taps preserve placement",
    );

    render(
      <Box
        data-rail-test
        sx={{
          position: "relative",
          width: 390,
          height: 500,
          ...mobileDrawerRailHitSx,
        }}
      >
        <Box data-test-drawer>
          <Box
            data-test-navigation
            sx={{ width: MOBILE_SESSION_DRAWER_WIDTH, height: 500 }}
          />
        </Box>
        <Box data-test-mask />
        <Box data-test-peek sx={{ position: "absolute", inset: 0 }} />
        <Box
          data-mobile-drawer-close="left"
          sx={{ position: "absolute", inset: 0 }}
        />
      </Box>,
    );
    const rail = document.querySelector<HTMLElement>("[data-rail-test]")!;
    const surface = document.querySelector<HTMLElement>("[data-test-peek]")!;
    const drawer = document.querySelector<HTMLElement>("[data-test-drawer]")!;
    const mask = document.querySelector<HTMLElement>("[data-test-mask]")!;
    let open = true;
    const binding = bindMobileSpatialDrawer({
      gestureTarget: rail,
      surface,
      drawer,
      drawerMask: mask,
      side: "left",
      phone: true,
      getOpen: () => open,
      setOpen: (value) => {
        open = value;
      },
    });
    try {
      for (const width of [320, 390, 430, 740]) {
        rail.style.width = `${width}px`;
        window.dispatchEvent(new Event("resize"));
        const expected = Math.min(480, width - 44);
        const navigation = document.querySelector<HTMLElement>(
          "[data-test-navigation]",
        )!;
        const close = document.querySelector<HTMLElement>(
          "[data-mobile-drawer-close]",
        )!;
        check(
          navigation.getBoundingClientRect().width === expected,
          `Rail width drift at ${width}`,
        );
        check(
          Math.abs(
            new DOMMatrix(getComputedStyle(surface).transform).m41 - expected,
          ) < 1,
          "Peek and rail width differ",
        );
        check(
          parseFloat(getComputedStyle(close).left) === expected,
          "Dismiss layer overlaps the wider rail",
        );
        check(
          close.getBoundingClientRect().width >= 44,
          "Dismiss target became too narrow",
        );
      }
      rail.style.width = "390px";
      window.dispatchEvent(new Event("resize"));
      const touch = (type: string, x: number) => {
        const event = new Event(type, { bubbles: true, cancelable: true });
        Object.defineProperty(event, "touches", {
          value: type === "touchcancel"
            ? []
            : [{ clientX: x, clientY: 100, identifier: 1, target: surface }],
        });
        surface.dispatchEvent(event);
      };
      touch("touchstart", 330);
      touch("touchmove", 300);
      check(
        Math.abs(new DOMMatrix(getComputedStyle(surface).transform).m41 - 316) <
          1,
        "Wider drawer no longer tracks the finger 1:1",
      );
      touch("touchcancel", 300);
      await new Promise((resolve) => setTimeout(resolve, 450));
      check(open, "Cancelling a close swipe closed the drawer");
      binding.settle(false);
      await new Promise((resolve) => setTimeout(resolve, 450));
      check(
        !open && !rail.hasAttribute("data-mobile-drawer-moving"),
        "Drawer did not release its gesture ownership",
      );
    } finally {
      binding.dispose();
    }
    results.push(
      "Wider rail, peek and dismissal hit area agree across resize; touch tracking is 1:1 and cancellation/settle release ownership",
    );
    if (preview) picker("parent");
    return results;
  } finally {
    if (!preview) {
      flushSync(() => root.unmount());
      container.remove();
    }
  }
}
