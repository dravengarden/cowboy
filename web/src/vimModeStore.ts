import { useSyncExternalStore } from "react";

// The live vim mode (normal / insert / visual / replace …) of the composer's
// editor, lifted to a tiny module store so the app-level status bar — which lives
// OUTSIDE the Composer, at the very bottom of the window like Zed / VSCode — can
// read what ComposerEditor's `vim-mode-change` produces. Only one composer is ever
// mounted, so a single global value is enough (no per-session keying). Ephemeral,
// not persisted; reactive via useSyncExternalStore like stickyStore.

let mode = "normal";
const listeners = new Set<() => void>();

export function setVimMode(next: string): void {
  if (next === mode) return;
  mode = next;
  for (const l of listeners) l();
}

export function useVimMode(): string {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => mode,
    () => "normal",
  );
}

// Status-bar colour per mode — insert is the "you can type" green, visual the
// selection amber; normal / anything else falls back to muted text.
export const VIM_MODE_COLOR: Record<string, string> = {
  insert: "success.main",
  visual: "warning.main",
};
