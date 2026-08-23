import { persisted, useStore } from "@cowboy/state-store";

// Composer expand preference, persisted + reactive (the ↗/↙ toggle writes it;
// the composer reads it) without prop-drilling — same shape as vimSetting.ts.
// Expanded swaps the auto-growing editor for a tall fixed editing area (Zed's
// expand affordance) for composing long messages. Per-device intent only;
// stored as the legacy "1"/"0" string so the value survives the state-store
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

// Drag-resized height of the EXPANDED editor (px), persisted per device. 0 = "no
// custom height yet" → fall back to the 48vh default. The top-edge resize handle
// (VSCode-terminal style) writes it; ComposerEditor reads it when expanded. Drag
// math + the collapse/expand auto-switch live in the Composer.
const height = persisted("cowboy:composer-height", 0, {
  serialize: (px) => String(Math.round(px)),
  deserialize: (s) => {
    const n = Number(s);
    return Number.isFinite(n) && n > 0 ? n : 0;
  },
});

export function useComposerHeight(): number {
  return useStore(height);
}

export function setComposerHeight(px: number): void {
  height.set(Math.round(px));
}
