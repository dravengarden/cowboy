interface QueryRoot {
  querySelector(selectors: string): unknown;
}

/**
 * The topmost Desktop overlay owns keyboard input before the workspace below.
 * Explicit scopes cover custom overlays; MUI Dialog/Popover roots make the
 * invariant automatic for ordinary product surfaces and menus.
 */
export function desktopOverlayOwnsShortcuts(root: QueryRoot): boolean {
  return root.querySelector(
    "[data-desktop-shortcut-scope='exclusive'], [role='dialog'][aria-modal='true'], .MuiPopover-root",
  ) !== null;
}

/**
 * The Machine browser's remote viewer owns every key while it has focus:
 * Space, the workspace prefix, Esc and browser shortcuts all belong to the
 * remote page, so the Desktop router (leaders included) stands aside. Only
 * the viewer's own release chord is handled, by the browser modal.
 */
export function remoteViewerOwnsKey(event: Pick<KeyboardEvent, "target">): boolean {
  const target = event.target as { closest?: (selector: string) => unknown } | null;
  return typeof target?.closest === "function" &&
    target.closest("[data-machine-browser-viewer]") !== null;
}
