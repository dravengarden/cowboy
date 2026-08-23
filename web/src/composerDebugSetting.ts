import { persisted, useStore } from "./components/state/store/mod.ts";

const composerDebug = persisted("cowboy:debug", false, {
  serialize: (on) => (on ? "1" : "0"),
  deserialize: (s) => s === "1",
});

export function useComposerDebugSetting(): boolean {
  return useStore(composerDebug);
}

export function getComposerDebugSetting(): boolean {
  return composerDebug.get();
}

export function setComposerDebugSetting(on: boolean): void {
  composerDebug.set(on);
}
