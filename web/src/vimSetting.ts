import { persisted, useStore } from "@cowboy/state-store";

// Vim-mode preference, persisted + reactive across the app (the composer reads
// it; the Settings toggle writes it) without prop-drilling. Vim itself is
// desktop-only — ComposerEditor gates the actual extension load on a precise-
// pointer device — so this is just the on/off intent. Stored as the legacy
// "1"/"0" string (format preserved via serialize/deserialize so existing prefs
// survive the state-store migration).
const vim = persisted("cowboy:vim", false, {
  serialize: (on) => (on ? "1" : "0"),
  deserialize: (s) => s === "1",
});

export function useVimSetting(): boolean {
  return useStore(vim);
}

export function getVimSetting(): boolean {
  return vim.get();
}

export function setVimSetting(on: boolean): void {
  vim.set(on);
}

// The user's Vim configuration (a vimrc subset, see desktop/vim/vimConfig.ts),
// kept beside the on/off intent on this device.
const vimConfig = persisted("cowboy:vim-config", "", {
  serialize: (text) => text,
  deserialize: (text) => text,
});

export function useVimConfigSource(): string {
  return useStore(vimConfig);
}

export function getVimConfigSource(): string {
  return vimConfig.get();
}

export function setVimConfigSource(text: string): void {
  vimConfig.set(text);
}

export function subscribeVimConfigSource(listener: () => void): () => void {
  return vimConfig.subscribe(listener);
}
