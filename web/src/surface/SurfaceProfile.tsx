import { createContext, useContext, useMemo } from "react";
import { useMediaQuery } from "@mui/material";
import { classifySurface, type SurfaceProfile } from "./profile";

export type { SurfaceInput, SurfaceKind, SurfaceProfile } from "./profile";

const SurfaceContext = createContext<SurfaceProfile | null>(null);

function nativeDesktopHost(): boolean {
  const global = globalThis as typeof globalThis & {
    __TAURI_INTERNALS__?: unknown;
  };
  if (global.__TAURI_INTERNALS__ !== undefined) return true;

  const nav = global.navigator;
  if (!nav) return false;
  const platform = nav.platform ?? "";
  // iPadOS commonly reports itself as Macintosh. Multiple touch points is the
  // reliable discriminator; do not turn an iPad with a trackpad into Desktop.
  if (/Mac/i.test(platform) && nav.maxTouchPoints > 1) return false;
  return /Mac|Win|Linux/i.test(platform);
}

export function SurfaceProvider(
  { children }: { children: React.ReactNode },
): React.JSX.Element {
  const finePointer = useMediaQuery("(pointer: fine)");
  const hover = useMediaQuery("(hover: hover)");
  const tabletWidth = useMediaQuery("(min-width: 600px)");
  const coarsePointerAvailable = useMediaQuery("(any-pointer: coarse)");
  const touchCapable = (globalThis.navigator?.maxTouchPoints ?? 0) > 0 ||
    coarsePointerAvailable;
  const profile = useMemo(
    () =>
      classifySurface({
        finePointer,
        hover,
        tabletWidth,
        touchCapable,
        desktopHost: nativeDesktopHost(),
      }),
    [finePointer, hover, tabletWidth, touchCapable],
  );
  return (
    <SurfaceContext.Provider value={profile}>
      {children}
    </SurfaceContext.Provider>
  );
}

export function useSurfaceProfile(): SurfaceProfile {
  const profile = useContext(SurfaceContext);
  if (!profile) {
    throw new Error("useSurfaceProfile must be used inside SurfaceProvider");
  }
  return profile;
}
