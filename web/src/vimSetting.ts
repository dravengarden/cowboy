import { useSyncExternalStore } from "react";

// Vim-mode preference, persisted in localStorage and reactive across the app
// (the composer reads it; the Settings toggle writes it) without prop-drilling.
// Vim itself is desktop-only — ComposerEditor gates the actual extension load
// on a precise-pointer device — so this is just the on/off intent.
const KEY = "cowboy:vim";
const EVENT = "cowboy:vim-change";

function getVim(): boolean {
  return globalThis.localStorage?.getItem(KEY) === "1";
}

function subscribe(onChange: () => void): () => void {
  globalThis.addEventListener?.(EVENT, onChange);
  globalThis.addEventListener?.("storage", onChange); // other tabs
  return () => {
    globalThis.removeEventListener?.(EVENT, onChange);
    globalThis.removeEventListener?.("storage", onChange);
  };
}

export function useVimSetting(): boolean {
  return useSyncExternalStore(subscribe, getVim, () => false);
}

export function setVimSetting(on: boolean): void {
  globalThis.localStorage?.setItem(KEY, on ? "1" : "0");
  globalThis.dispatchEvent?.(new Event(EVENT));
}
