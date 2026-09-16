import { persisted, useStore } from "@cowboy/state-store";

// Obsidian's "Source mode" for the composer. The DOCUMENT never changes — it is
// literal GFM either way — only whether mdlive renders it inline. In source mode
// every `**`, `# `, `> `, `- [ ]` marker stays visible as ordinary text while you
// edit; live preview (the default) keeps rendering it and revealing the markers
// on the active line.
//
// One global, persisted preference rather than a per-session or per-surface
// flag, for the same reason Vim mode is one (vimSetting.ts): it is an editing
// preference, and the compact composer, the fullscreen editor, and the
// queue/draft editors must never disagree about how the same markdown is shown.
// Per device (localStorage), stored as "1"/"0" like the other composer prefs.
const sourceMode = persisted("cowboy:source-mode", false, {
  serialize: (on) => (on ? "1" : "0"),
  deserialize: (s) => s === "1",
});

export function useComposerSourceMode(): boolean {
  return useStore(sourceMode);
}

export function getComposerSourceMode(): boolean {
  return sourceMode.get();
}

export function setComposerSourceMode(on: boolean): void {
  sourceMode.set(on);
}

/** Flip the mode and return the new value (the toolbar/command/settings entry). */
export function toggleComposerSourceMode(): boolean {
  const next = !sourceMode.get();
  sourceMode.set(next);
  return next;
}
