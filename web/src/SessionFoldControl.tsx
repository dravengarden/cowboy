// The Sessions fold button (docs/sessions-folders.md). One control alternates
// between a focused view (only the current session's folder path open, its
// row in view) and every folder open. The action derives from the fold state
// and whether the current row is really on screen, so a manual fold or
// scrolling away re-arms Focus and the icon always names what the next press
// does.

import {
  type ReactNode,
  type RefObject,
  useEffect,
  useLayoutEffect,
  useState,
} from "react";
import { Box } from "@mui/material";
import { alpha, type SxProps, type Theme } from "@mui/material/styles";
import {
  MyLocationOutlined,
  UnfoldLess,
  UnfoldMore,
} from "@mui/icons-material";
import type { SessionFoldersValue } from "./sessionFolders";
import {
  foldersOffSessionPath,
  type SessionFoldAction,
  sessionFoldAction,
  type SessionTree,
} from "./sessionTree";

const rowSelector = (key: string): string =>
  `[data-desktop-item="${CSS.escape(key)}"]`;

/** The three glyphs share one slot and cross-fade on a state change. */
export function SessionFoldIcon(
  { action }: { readonly action: SessionFoldAction | null },
): ReactNode {
  const shown = action ?? "focus";
  const glyphs: readonly [SessionFoldAction, ReactNode][] = [
    ["focus", <UnfoldLess fontSize="inherit" />],
    ["expand", <UnfoldMore fontSize="inherit" />],
    [
      "locate",
      <MyLocationOutlined
        fontSize="inherit"
        sx={{ transform: "scale(0.88)" }}
      />,
    ],
  ];
  return (
    <Box
      component="span"
      aria-hidden
      data-session-fold-icon={shown}
      sx={{
        display: "inline-grid",
        placeItems: "center",
        width: "1em",
        height: "1em",
        fontSize: "1.3em",
        "& > span": {
          gridArea: "1 / 1",
          display: "inline-flex",
          transition:
            "opacity 160ms ease, transform 240ms cubic-bezier(0.22, 1, 0.36, 1)",
        },
        "@media (prefers-reduced-motion: reduce)": {
          "& > span": { transition: "none" },
        },
      }}
    >
      {glyphs.map(([key, glyph]) => (
        <Box
          key={key}
          component="span"
          sx={{
            opacity: key === shown ? 1 : 0,
            transform: key === shown ? "scale(1)" : "scale(0.6)",
          }}
        >
          {glyph}
        </Box>
      ))}
    </Box>
  );
}

const locateKeyframes = {
  "0%, 35%": { boxShadow: "inset 0 0 0 2px var(--cowboy-locate-ring)" },
  "100%": { boxShadow: "inset 0 0 0 2px transparent" },
};

export interface SessionFoldControl {
  readonly action: SessionFoldAction | null;
  readonly label: string;
  readonly icon: ReactNode;
  readonly run: () => void;
  /** Row styles for the brief ring that marks the located session. */
  readonly pulseSx: (key: string) => SxProps<Theme> | undefined;
}

export function useSessionFoldControl({
  listRef,
  tree,
  folders,
  collapsed,
  setCollapsed,
  activeId,
  bottomInset,
  onLocate,
}: {
  readonly listRef: RefObject<HTMLElement | null>;
  readonly tree: SessionTree;
  readonly folders: SessionFoldersValue;
  readonly collapsed: ReadonlySet<string>;
  readonly setCollapsed: (next: ReadonlySet<string>) => void;
  readonly activeId: string | null;
  /** List pixels hidden under floating chrome; a row there is not in view. */
  readonly bottomInset: number;
  /** Scroll the (now visible) current row into view. */
  readonly onLocate: (key: string) => void;
}): SessionFoldControl {
  const [activeInView, setActiveInView] = useState(false);
  const [pulse, setPulse] = useState<{ key: string; n: number } | null>(null);
  const [anchor, setAnchor] = useState<{ key: string; top: number } | null>(
    null,
  );
  const activeShown = activeId !== null &&
    tree.rows.some((row) =>
      row.kind === "session" && row.session.id === activeId
    );
  useEffect(() => {
    const list = listRef.current;
    const item = activeId && activeShown
      ? list?.querySelector<HTMLElement>(rowSelector(activeId))
      : null;
    if (!list || !item || typeof IntersectionObserver === "undefined") {
      setActiveInView(false);
      return undefined;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        const entry = entries.at(-1);
        if (entry) setActiveInView(entry.intersectionRatio >= 0.98);
      },
      {
        root: list,
        threshold: [0, 0.98],
        rootMargin: `0px 0px -${String(bottomInset)}px 0px`,
      },
    );
    observer.observe(item);
    return () => observer.disconnect();
  }, [activeId, activeShown, bottomInset, listRef]);
  useLayoutEffect(() => {
    if (!anchor) return;
    const list = listRef.current;
    const item = list?.querySelector<HTMLElement>(rowSelector(anchor.key));
    if (list && item) {
      list.scrollTop += item.getBoundingClientRect().top -
        list.getBoundingClientRect().top - anchor.top;
    }
    setAnchor(null);
  }, [anchor, listRef, tree]);
  useEffect(() => {
    if (!pulse) return undefined;
    const timer = setTimeout(() => setPulse(null), 1100);
    return () => clearTimeout(timer);
  }, [pulse]);

  const action = sessionFoldAction(
    tree,
    folders,
    collapsed,
    activeId,
    activeInView,
  );
  const label = action === "expand"
    ? "Expand all folders"
    : action === "locate"
    ? "Show current session"
    : activeId
    ? "Focus current session"
    : "Collapse all folders";
  // Keep the row the reader is looking at (the current session when it is in
  // view, else the topmost visible row) fixed while folders open or close.
  const readAnchor = (): { key: string; top: number } | null => {
    const list = listRef.current;
    if (!list) return null;
    const top = list.getBoundingClientRect().top;
    const items = [
      ...list.querySelectorAll<HTMLElement>("[data-desktop-item]"),
    ];
    const item =
      (activeInView
        ? items.find((el) => el.dataset.desktopItem === activeId)
        : undefined) ??
        items.find((el) => el.getBoundingClientRect().bottom > top);
    const key = item?.dataset.desktopItem;
    return item && key
      ? { key, top: item.getBoundingClientRect().top - top }
      : null;
  };
  const run = (): void => {
    if (!action) return;
    if (action === "expand" || !activeId) setAnchor(readAnchor());
    if (action === "expand") {
      setCollapsed(new Set());
      return;
    }
    if (action === "focus") {
      setCollapsed(new Set(foldersOffSessionPath(tree, folders, activeId)));
    }
    if (activeId) {
      onLocate(activeId);
      setPulse((previous) => ({ key: activeId, n: (previous?.n ?? 0) + 1 }));
    }
  };
  const pulseSx = (key: string): SxProps<Theme> | undefined =>
    pulse?.key === key
      ? {
        "--cowboy-locate-ring": (t: Theme) =>
          alpha(t.palette.primary.main, 0.75),
        // Alternating names restart the ring on a repeated press.
        animation: `session-locate-${String(pulse.n % 2)} 1000ms ease-out`,
        "@keyframes session-locate-0": locateKeyframes,
        "@keyframes session-locate-1": locateKeyframes,
        "@media (prefers-reduced-motion: reduce)": { animation: "none" },
      }
      : undefined;
  return {
    action,
    label,
    icon: <SessionFoldIcon action={action} />,
    run,
    pulseSx,
  };
}
