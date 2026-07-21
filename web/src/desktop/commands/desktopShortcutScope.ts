interface QueryRoot {
  querySelector(selectors: string): unknown;
}

/** Popovers with their own visible key map temporarily own unmodified keys. */
export function desktopOverlayOwnsShortcuts(root: QueryRoot): boolean {
  return root.querySelector("[data-desktop-shortcut-scope='exclusive']") !== null;
}
