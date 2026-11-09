/** How the viewer's device takes input and how it is held, the two facts
 * that choose the remote screen mode and the keyboard path. Touch follows
 * neko's rule: a device with touch points and neither a fine pointer nor
 * hover is a touch device; without media queries the surface profile
 * decides. Orientation comes from the screen, never from the frame, so the
 * soft keyboard shrinking the frame never reads as a rotation. */
import { useMediaQuery } from "@mui/material";

export interface PointerEnvironment {
  /** `ontouchstart` exists or the device reports touch points. */
  touchPoints: boolean;
  finePointer: boolean;
  hover: boolean;
}

type Scope = {
  matchMedia?: (query: string) => { matches: boolean };
  navigator?: { maxTouchPoints?: number };
  screen?: { orientation?: { type?: string } };
  orientation?: unknown;
  innerWidth?: number;
  innerHeight?: number;
};

/** neko's touch-device rule (`client.ts`, `isTouchDevice`). */
export function touchInput(
  environment: PointerEnvironment | undefined,
  fallback: boolean,
): boolean {
  if (!environment) return fallback;
  return environment.touchPoints && !environment.finePointer && !environment.hover;
}

/** The pointer facts of this page, or `undefined` without media queries. */
export function pointerEnvironment(
  scope: Scope = globalThis as unknown as Scope,
): PointerEnvironment | undefined {
  if (typeof scope.matchMedia !== "function") return undefined;
  return {
    touchPoints: "ontouchstart" in scope || (scope.navigator?.maxTouchPoints ?? 0) > 0,
    finePointer: scope.matchMedia("(pointer: fine)").matches,
    hover: scope.matchMedia("(hover: hover)").matches,
  };
}

/** Whether the device is held upright: the Screen Orientation API, else
 * the legacy `window.orientation` angle, else the window's own shape. */
export function devicePortrait(scope: Scope = globalThis as unknown as Scope): boolean {
  const type = scope.screen?.orientation?.type;
  if (typeof type === "string") return type.startsWith("portrait");
  if (typeof scope.orientation === "number") return scope.orientation % 180 === 0;
  return !((scope.innerWidth ?? 0) > (scope.innerHeight ?? 0));
}

/** Call `listener` on every device rotation; returns the unsubscribe. */
export function onOrientationChange(listener: () => void): () => void {
  const orientation = (globalThis as { screen?: { orientation?: EventTarget } }).screen
    ?.orientation;
  const target = globalThis as { addEventListener?: EventTarget["addEventListener"] };
  orientation?.addEventListener?.("change", listener);
  target.addEventListener?.("orientationchange", listener);
  return () => {
    orientation?.removeEventListener?.("change", listener);
    (
      globalThis as { removeEventListener?: EventTarget["removeEventListener"] }
    ).removeEventListener?.("orientationchange", listener);
  };
}

/** Touch input for the viewer, following pointer and hover changes (a
 * tablet gaining a trackpad); `fallback` is the surface profile's answer. */
export function useTouchInput(fallback: boolean): boolean {
  const finePointer = useMediaQuery("(pointer: fine)", { noSsr: true });
  const hover = useMediaQuery("(hover: hover)", { noSsr: true });
  const environment = pointerEnvironment();
  return touchInput(environment && { ...environment, finePointer, hover }, fallback);
}
