/** Shared chrome: root-relative on Desktop, original pixel size on touch.
 * SurfaceProvider publishes the unit on <html>, including portal surfaces.
 * Layout dimensions and Mobile targets must not be converted indiscriminately. */
export function desktopSize(px: number): string {
  return `calc(${px / 16} * var(--cowboy-desktop-size-unit, 16px))`;
}

/** A functional icon glyph: root-relative on every surface, so it follows the
 * global font size on Mobile as well as Desktop (where it equals
 * `desktopSize`). Hit targets and layout keep `desktopSize`; Transcript
 * activity marks keep their authored pixels (docs/architecture/09-frontend.md). */
export function glyphSize(px: number): string {
  return `${px / 16}rem`;
}
