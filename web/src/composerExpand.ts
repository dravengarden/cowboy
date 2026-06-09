import { persisted, useStore } from "./_store/mod.ts";

// Composer expand preference, persisted + reactive (the ↗/↙ toggle writes it;
// the composer reads it) without prop-drilling — same shape as vimSetting.ts.
// Expanded swaps the auto-growing editor for a tall fixed editing area (Zed's
// expand affordance) for composing long messages. Per-device intent only;
// stored as the legacy "1"/"0" string so the value survives the @shared-utils
// store migration like the other cowboy prefs.
const expanded = persisted("cowboy:composer-expanded", false, {
  serialize: (on) => (on ? "1" : "0"),
  deserialize: (s) => s === "1",
});

export function useComposerExpanded(): boolean {
  return useStore(expanded);
}

export function setComposerExpanded(on: boolean): void {
  expanded.set(on);
}

export function toggleComposerExpanded(): void {
  expanded.set(!expanded.get());
}
