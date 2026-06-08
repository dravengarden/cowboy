import { persisted, useStore } from "./_store/mod.ts";

// App-wide "Frosted sheets" preference. When on, every modal BottomSheet uses
// DetentSheet's translucent 磨砂玻璃 variant (a milky tint over blur+saturate, with
// a lighter scrim so the page diffuses through) instead of the solid surface.
// A per-device LOOK preference (not cross-device synced), persisted + reactive
// across tabs via @shared-utils/store. Default ON — the frosted material is the
// intended look; toggle off in Settings for a solid surface.
const frosted = persisted<boolean>("cowboy:frosted-sheets", true, {
  serialize: (b) => (b ? "1" : "0"),
  deserialize: (s) => s !== "0",
});

export function useFrostedSheets(): boolean {
  return useStore(frosted);
}

export function setFrostedSheets(on: boolean): void {
  frosted.set(on);
}
