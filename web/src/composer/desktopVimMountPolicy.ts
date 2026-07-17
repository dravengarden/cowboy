import type { SurfaceKind } from "../surface/profile";

export function desktopVimMountPolicy(
  surfaceKind: SurfaceKind,
  vimRequested: boolean,
  runtimeReady: boolean,
  runtimeFailed = false,
): { awaitingRuntime: boolean; enableVim: boolean } {
  const wantsDesktopVim = surfaceKind === "desktop" && vimRequested;
  return {
    awaitingRuntime: wantsDesktopVim && !runtimeReady && !runtimeFailed,
    enableVim: wantsDesktopVim && runtimeReady,
  };
}
