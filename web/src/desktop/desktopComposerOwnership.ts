/** Prompt owns typed keys only while its workspace region is the focused one. */
export function desktopComposerOwnsWorkspace(
  focusedRegion: string | null,
): boolean {
  return focusedRegion === "prompt.composer";
}

export function desktopDefaultRegionForPane(
  pane: string | undefined,
): string | null {
  if (pane === "conversation") return "conversation.transcript";
  if (pane === "sessions") return "sessions.list";
  if (pane === "prompt") return "prompt.composer";
  return null;
}

export function desktopVimSinkShouldHandleKeys(input: {
  focusedRegion: string | null;
  targetIsVimSink: boolean;
  targetRegion: string | null;
}): boolean {
  if (!input.targetIsVimSink) return false;
  return input.targetRegion !== null && input.targetRegion === input.focusedRegion;
}

export function desktopShouldBlockStaleVimSink(input: {
  focusedRegion: string | null;
  targetIsVimSink: boolean;
  targetRegion: string | null;
}): boolean {
  return input.targetIsVimSink && !desktopVimSinkShouldHandleKeys(input);
}

export function desktopRegionFromPointerTarget(
  target: EventTarget | null,
): string | null {
  if (!(target instanceof Element)) return null;
  const region = target.closest<HTMLElement>("[data-desktop-region]")
    ?.dataset.desktopRegion;
  if (region) return region;
  const pane = target.closest<HTMLElement>("[data-desktop-pane]")
    ?.dataset.desktopPane;
  return desktopDefaultRegionForPane(pane);
}

export function desktopPointerLeftComposer(
  target: EventTarget | null,
  active: Element | null,
): boolean {
  if (!(target instanceof Element) || !(active instanceof Element)) return false;
  const composer = active.closest("[data-desktop-region='prompt.composer']");
  if (!composer) return false;
  return !composer.contains(target);
}
