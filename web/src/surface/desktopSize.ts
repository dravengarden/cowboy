/** Shared chrome: root-relative on Desktop, original pixel size on touch.
 * SurfaceProvider publishes the unit on <html>, including portal surfaces.
 * Layout dimensions and Mobile targets must not be converted indiscriminately. */
export function desktopSize(px: number): string {
  return `calc(${px / 16} * var(--cowboy-desktop-size-unit, 16px))`;
}
