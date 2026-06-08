import { persisted, useStore } from "./_store/mod.ts";

// Vim-mode preference, persisted + reactive across the app (the composer reads
// it; the Settings toggle writes it) without prop-drilling. Vim itself is
// desktop-only — ComposerEditor gates the actual extension load on a precise-
// pointer device — so this is just the on/off intent. Stored as the legacy
// "1"/"0" string (format preserved via serialize/deserialize so existing prefs
// survive the @shared-utils/store migration).
const vim = persisted("cowboy:vim", false, {
  serialize: (on) => (on ? "1" : "0"),
  deserialize: (s) => s === "1",
});

export function useVimSetting(): boolean {
  return useStore(vim);
}

export function setVimSetting(on: boolean): void {
  vim.set(on);
}
