/** Leading glyph geometry for Desktop embedded controls. Use explicit geometry
 * because WebKit/Chromium minimum-font-size preferences may clamp rem-based SVG
 * font sizes even while Cowboy's global scale is below 100%. */
export function desktopEmbeddedControlIconSx() {
  return {
    fontSize: "calc(20px * var(--cowboy-font-scale, 1))",
    width: "calc(20px * var(--cowboy-font-scale, 1))",
    height: "calc(20px * var(--cowboy-font-scale, 1))",
    flexShrink: 0,
  } as const;
}
