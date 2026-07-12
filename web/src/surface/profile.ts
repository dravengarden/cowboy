export type SurfaceKind = "mobile" | "tablet" | "desktop";
export type SurfaceInput = "touch" | "hybrid" | "keyboard-pointer";

export interface SurfaceProfile {
  kind: SurfaceKind;
  input: SurfaceInput;
  touchCapable: boolean;
  finePointer: boolean;
  hover: boolean;
}

export function classifySurface({
  finePointer,
  hover,
  tabletWidth,
  touchCapable,
  desktopHost,
}: {
  finePointer: boolean;
  hover: boolean;
  tabletWidth: boolean;
  touchCapable: boolean;
  desktopHost: boolean;
}): SurfaceProfile {
  const desktop = finePointer && hover && (desktopHost || !touchCapable);
  const kind: SurfaceKind = desktop
    ? "desktop"
    : tabletWidth
    ? "tablet"
    : "mobile";
  const input: SurfaceInput = desktop
    ? (touchCapable ? "hybrid" : "keyboard-pointer")
    : "touch";
  return { kind, input, touchCapable, finePointer, hover };
}
