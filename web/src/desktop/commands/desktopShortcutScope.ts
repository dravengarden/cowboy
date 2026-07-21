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
