import type { SurfaceKind } from "../surface/profile";

export type DesktopVimRuntimeState = "pending" | "ready" | "failed";

export function shouldPreloadDesktopVim(
  surfaceKind: SurfaceKind,
  vimRequested: boolean,
  runtimeState: DesktopVimRuntimeState,
): boolean {
  return surfaceKind === "desktop" && vimRequested && runtimeState === "pending";
}

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
