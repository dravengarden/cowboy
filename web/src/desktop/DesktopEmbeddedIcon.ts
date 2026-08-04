/** Leading glyph geometry for Desktop embedded controls. MUI's `small` icon
 * resolves to a fixed 20px, so it cannot follow Cowboy's root font scale. */
export function desktopEmbeddedControlIconSx() {
  return {
    fontSize: "1.25rem",
    flexShrink: 0,
  } as const;
}
