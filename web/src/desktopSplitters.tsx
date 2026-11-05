import { useEffect, useRef, useState } from "react";
import { clampComposerColWidth, composerColWidthStore, DESKTOP_SESSIONS_DRAWER_TOGGLE_EVENT, clampPromptRatio, collapsedSessionsPromptRatioStore } from "./desktopLayout";
import { persisted } from "@cowboy/state-store";
import { useOptionalDesktopWorkspace } from "./desktop/DesktopWorkspaceController";
import { DESKTOP_SPLITTER_ADJUST_EVENT, splitterAdjustment } from "./desktop/desktopSplitterKeyboard";

// Desktop sidebar width: a user-draggable pixel width (VSCode-style divider),
// persisted in localStorage. The bounds keep both panes usable — 240px floor
// (a list row stays readable), 480px ceiling (wider and the list looks empty
// next to the transcript). 300px default sits inside the old fluid
// `clamp(240px, 22vw, 360px)` it replaces. Resize is desktop-only: below the
// `lg` breakpoint the sidebar is a full-width top Drawer (touch layout), which
// has no divider — so neither bound nor handle applies there.
// App-wide bottom status bar (Zed / VSCode style): a thin strip at the very
// bottom of the window. DESKTOP ONLY + only when vim is on (its sole status today
export const SIDEBAR_MIN = 240;

export const SIDEBAR_MAX = 480;

export const SIDEBAR_DEFAULT = 288;

export function clampSidebarWidth(px: number): number {
    return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, px));
}

// Persisted (per-device) sidebar width — seeded into local state for the live
// drag, written back on settle. Re-clamped on read in case the bounds changed
// since it was stored; default when unset/unparseable. String(px) format kept.
export const sidebarWidthStore = persisted("cowboy:sidebar-width", SIDEBAR_DEFAULT, {
    serialize: String,
    deserialize: (raw) => {
        const n = Number.parseInt(raw, 10);
        return Number.isFinite(n) ? clampSidebarWidth(n) : SIDEBAR_DEFAULT;
    },
});

/** Desktop splitter state: the Sessions and Prompt column widths, the
 *  collapsed-Sessions prompt ratio, live-drag flags with their body cursor,
 *  drag-to-collapse previews, and keyboard splitter adjustment. */
export function useDesktopSplitters({
    surface,
    sessionsCollapsed,
    desktopWorkspace,
    setDrawerOpen,
}: {
    surface: "desktop" | "touch";
    sessionsCollapsed: boolean;
    desktopWorkspace: ReturnType<typeof useOptionalDesktopWorkspace>;
    setDrawerOpen: React.Dispatch<React.SetStateAction<boolean>>;
}) {
// Desktop sidebar width + live-drag flag. Width is a pixel value (not the
// old fluid clamp) so the divider can set it directly; `resizing` drives
// the handle's active highlight and a body-wide drag cursor / no-select.
const [sidebarWidth, setSidebarWidth] = useState<number>(sidebarWidthStore.get);
const [resizing, setResizing] = useState(false);
// Persist after a drag settles, not on every pointermove (localStorage is
// synchronous — writing per pixel would stutter the drag). The ref carries
// the latest dragged width into pointerup without re-binding listeners.
const widthRef = useRef(sidebarWidth);
widthRef.current = sidebarWidth;
// While dragging the divider, force the col-resize cursor and kill text
// selection document-wide — otherwise the pointer flickers to a text caret
// and a fast drag highlights the transcript. Restored on release/unmount.
useEffect(() => {
    if (!resizing) return undefined;
    const { body } = document;
    const prevCursor = body.style.cursor;
    const prevSelect = body.style.userSelect;
    body.style.cursor = "col-resize";
    body.style.userSelect = "none";
    return (): void => {
        body.style.cursor = prevCursor;
        body.style.userSelect = prevSelect;
    };
}, [resizing]);
// Two-pane working area (plus the Sessions rail when space permits). Desktop
// remains split even when the rail collapses: a narrower desktop is still a
// keyboard-first production surface, not Mobile. Mobile never reads this
// preference and always owns its separate overlay composer path.
const splitActive = surface === "desktop";
// Composer-column width + live-drag, mirroring the sidebar splitter: a local
// value during the drag (persisted on release, not per-pixel) backed by the
// global composer-col-width store. `colResizing` drives the body drag cursor.
const [colWidth, setColWidth] = useState<number>(composerColWidthStore.get);
const [colResizing, setColResizing] = useState(false);
const colWidthRef = useRef(colWidth);
colWidthRef.current = colWidth;
// Drag-to-collapse preview: the pane a splitter release would fold away.
const [sessionsCollapseIntent, setSessionsCollapseIntent] = useState(false);
const [workCollapseIntent, setWorkCollapseIntent] =
    useState<"prompt" | "conversation" | null>(null);
// With Sessions collapsed, Prompt and Conversation split by a remembered
// ratio (even by default) rather than the pixel width used beside the
// full Sessions list.
const [promptRatio, setPromptRatio] = useState<number>(collapsedSessionsPromptRatioStore.get);
const promptRatioRef = useRef(promptRatio);
promptRatioRef.current = promptRatio;
const sessionsCollapsedRef = useRef(sessionsCollapsed);
sessionsCollapsedRef.current = sessionsCollapsed;
// Compact Desktop: the Sessions collapse command opens/closes the drawer.
useEffect(() => {
    if (surface !== "desktop") return undefined;
    const onToggle = (): void => setDrawerOpen((open) => !open);
    globalThis.addEventListener(DESKTOP_SESSIONS_DRAWER_TOGGLE_EVENT, onToggle);
    return (): void =>
        globalThis.removeEventListener(DESKTOP_SESSIONS_DRAWER_TOGGLE_EVENT, onToggle);
}, [surface]);
useEffect(() => {
    if (!desktopWorkspace) return undefined;
    const onKeyboardResize = (event: Event): void => {
        const adjustment = splitterAdjustment(event);
        if (adjustment?.splitter === "sessions-prompt") {
            setSidebarWidth((current) => {
                const next = clampSidebarWidth(current + adjustment.delta);
                widthRef.current = next;
                sidebarWidthStore.set(next);
                return next;
            });
        } else if (
            adjustment?.splitter === "prompt-conversation" && sessionsCollapsedRef.current
        ) {
            const width = document.querySelector<HTMLElement>(
                "[data-desktop-splitter='prompt-conversation']",
            )?.parentElement?.getBoundingClientRect().width ?? 0;
            if (width > 0) {
                const next = clampPromptRatio(promptRatioRef.current + adjustment.delta / width);
                promptRatioRef.current = next;
                setPromptRatio(next);
                collapsedSessionsPromptRatioStore.set(next);
            }
        } else if (adjustment?.splitter === "prompt-conversation") {
            setColWidth((current) => {
                const next = clampComposerColWidth(current + adjustment.delta);
                colWidthRef.current = next;
                composerColWidthStore.set(next);
                return next;
            });
        }
    };
    globalThis.addEventListener(DESKTOP_SPLITTER_ADJUST_EVENT, onKeyboardResize);
    return (): void =>
        globalThis.removeEventListener(
            DESKTOP_SPLITTER_ADJUST_EVENT,
            onKeyboardResize,
        );
}, [desktopWorkspace]);
useEffect(() => {
    if (!colResizing) return undefined;
    const { body } = document;
    const prevCursor = body.style.cursor;
    const prevSelect = body.style.userSelect;
    body.style.cursor = "col-resize";
    body.style.userSelect = "none";
    return (): void => {
        body.style.cursor = prevCursor;
        body.style.userSelect = prevSelect;
    };
}, [colResizing]);
    return {
        sidebarWidth,
        setSidebarWidth,
        resizing,
        setResizing,
        widthRef,
        splitActive,
        colWidth,
        setColWidth,
        colResizing,
        setColResizing,
        colWidthRef,
        sessionsCollapseIntent,
        setSessionsCollapseIntent,
        workCollapseIntent,
        setWorkCollapseIntent,
        promptRatio,
        setPromptRatio,
        promptRatioRef,
    };
}
