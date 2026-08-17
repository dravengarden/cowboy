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

export function desktopEditorMountFocusPolicy(
  focusEndOnMount: boolean,
  awaitingRuntime: boolean,
  documentLength: number,
): { focusOnMount: boolean; initialSelection?: number } {
  if (!focusEndOnMount) return { focusOnMount: false };
  return {
    // A pending Vim editor is a disabled placeholder that will be replaced.
    // Focusing it loses keyboard ownership when the interactive editor mounts.
    focusOnMount: !awaitingRuntime,
    initialSelection: documentLength,
  };
}
