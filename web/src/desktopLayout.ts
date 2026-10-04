import { persisted, useStore } from "@cowboy/state-store";

// Width (px) of the composer column in split mode — persisted, clamped. Global
// (like the session sidebar width), not per-session.
export const COMPOSER_COL_MIN = 320;
// Rendered floors of the two Desktop work panes (see DesktopWorkspace).
export const DESKTOP_PROMPT_MIN = 360;
export const DESKTOP_CONVERSATION_MIN = 520;
export const COMPOSER_COL_MAX = 720;
const COMPOSER_COL_DEFAULT = 440;
export const READING_QUESTIONS_MIN = 240;
export const READING_QUESTIONS_MAX = 480;
const READING_QUESTIONS_DEFAULT = 320;

function clampColWidth(px: number): number {
  return Math.min(COMPOSER_COL_MAX, Math.max(COMPOSER_COL_MIN, Math.round(px)));
}

// Raw store exported so the splitter drag can mirror the sidebar-resize pattern
// (seed from `.get`, persist on pointerup via `.set`) — per-pixel `.set` during a
// drag would thrash localStorage, so the drag keeps a local value and commits once.
export const composerColWidthStore = persisted<number>(
  "cowboy:composer-col-width",
  COMPOSER_COL_DEFAULT,
  {
    serialize: (n) => String(Math.round(n)),
    deserialize: (s) => {
      const n = Number(s);
      return Number.isFinite(n) ? clampColWidth(n) : COMPOSER_COL_DEFAULT;
    },
  },
);

export function useComposerColWidth(): number {
  return useStore(composerColWidthStore);
}

export function clampComposerColWidth(px: number): number {
  return clampColWidth(px);
}

export function clampReadingQuestionsWidth(px: number): number {
  return Math.min(
    READING_QUESTIONS_MAX,
    Math.max(READING_QUESTIONS_MIN, Math.round(px)),
  );
}

export const readingQuestionsWidthStore = persisted<number>(
  "cowboy:reading-questions-width",
  READING_QUESTIONS_DEFAULT,
  {
    serialize: (n) => String(Math.round(n)),
    deserialize: (s) => {
      const n = Number(s);
      return Number.isFinite(n)
        ? clampReadingQuestionsWidth(n)
        : READING_QUESTIONS_DEFAULT;
    },
  },
);

// Desktop pane collapse. One global layout preference (like the widths above),
// never per session: hiding Sessions is a statement about the screen, not about
// one conversation. Prompt and Conversation are the work surface, so at least
// one of them always stays expanded; Sessions may collapse freely.
export type DesktopCollapsiblePane = "sessions" | "prompt" | "conversation";

export interface DesktopCollapsedPanes {
  sessions: boolean;
  prompt: boolean;
  conversation: boolean;
}

export const DESKTOP_PANES_EXPANDED: DesktopCollapsedPanes = {
  sessions: false,
  prompt: false,
  conversation: false,
};

// The same width floor App uses to move Sessions into its drawer. Below it the
// Sessions toggle opens/closes that drawer instead of the in-flow rail.
export const DESKTOP_COMPACT_WIDTH_QUERY = "(max-width:1099px)";

/** Repair any stored combination that would leave no work pane visible. */
export function normalizeCollapsedPanes(
  panes: DesktopCollapsedPanes,
): DesktopCollapsedPanes {
  return panes.prompt && panes.conversation
    ? { ...panes, conversation: false }
    : panes;
}

/**
 * Collapse or expand one pane. Collapsing the last visible work pane swaps it
 * with its hidden sibling, so the shortcut always does something visible and
 * the workspace never becomes empty.
 */
export function withPaneCollapsed(
  panes: DesktopCollapsedPanes,
  pane: DesktopCollapsiblePane,
  collapsed: boolean,
): DesktopCollapsedPanes {
  if (panes[pane] === collapsed) return panes;
  const next = { ...panes, [pane]: collapsed };
  if (collapsed && pane === "prompt") next.conversation = false;
  if (collapsed && pane === "conversation") next.prompt = false;
  return next;
}

export function togglePaneCollapsed(
  panes: DesktopCollapsedPanes,
  pane: DesktopCollapsiblePane,
): DesktopCollapsedPanes {
  return withPaneCollapsed(panes, pane, !panes[pane]);
}

export function parseCollapsedPanes(raw: string): DesktopCollapsedPanes {
  try {
    const value = JSON.parse(raw) as Record<string, unknown> | null;
    if (typeof value !== "object" || value === null) return DESKTOP_PANES_EXPANDED;
    return normalizeCollapsedPanes({
      sessions: value.sessions === true,
      prompt: value.prompt === true,
      conversation: value.conversation === true,
    });
  } catch {
    return DESKTOP_PANES_EXPANDED;
  }
}

export const desktopCollapsedPanesStore = persisted<DesktopCollapsedPanes>(
  "cowboy:desktop-collapsed-panes",
  DESKTOP_PANES_EXPANDED,
  {
    serialize: (panes) => JSON.stringify(normalizeCollapsedPanes(panes)),
    deserialize: parseCollapsedPanes,
  },
);

export function useDesktopCollapsedPanes(): DesktopCollapsedPanes {
  return useStore(desktopCollapsedPanesStore);
}

/**
 * Drag-to-collapse: a splitter dragged this far past a pane's minimum width
 * snaps that pane closed on release (VS Code's sidebar behaviour). The margin
 * keeps an ordinary "make it as small as possible" drag from hiding the pane.
 */
export const DESKTOP_DRAG_COLLAPSE_MARGIN = 96;

export function dragCollapses(rawWidth: number, minWidth: number): boolean {
  return rawWidth < minWidth - DESKTOP_DRAG_COLLAPSE_MARGIN;
}

// Compact Desktop keeps Sessions in its top drawer. The Sessions collapse
// command asks App to toggle that drawer rather than the in-flow rail.
export const DESKTOP_SESSIONS_DRAWER_TOGGLE_EVENT = "cowboy:desktop-sessions-drawer-toggle";
