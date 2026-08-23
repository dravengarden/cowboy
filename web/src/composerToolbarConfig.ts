// Which composer commands appear in the fullscreen markdown toolbar, and in what
// order — the persisted, user-curatable config the toolbar renders from (the
// Obsidian "Manage toolbar options" model). Per device (localStorage), same
// pattern as desktopLayout.ts / readingSettings.ts. The phase-2 settings sheet
// edits this list (add / remove / drag-reorder); until then it's the default.
import { persisted, useStore } from "./components/state/store/mod.ts";
import { COMPOSER_COMMANDS_BY_ID } from "./composerCommands";
import {
  DEFAULT_COMPOSER_TOOLBAR,
  normalizeComposerToolbarOrder,
} from "./composerToolbarModel";

export { DEFAULT_COMPOSER_TOOLBAR } from "./composerToolbarModel";

// The keyboard-nearest bar starts with the highest-frequency editing actions.
// Paste is injected immediately after Redo by MobileComposerFormatActions; the
// remaining commands continue horizontally in this persisted order. Attach is
// fixed in the message-action bar and therefore is not duplicated by default.
export function normalizeComposerToolbar(value: unknown): string[] {
  return normalizeComposerToolbarOrder(
    value,
    (id) => id in COMPOSER_COMMANDS_BY_ID,
  );
}

const store = persisted<string[]>(
  "cowboy:composer-toolbar",
  [...DEFAULT_COMPOSER_TOOLBAR],
  {
    serialize: (v) => JSON.stringify(v),
    deserialize: (s) => {
      try {
        return normalizeComposerToolbar(JSON.parse(s));
      } catch {
        return [...DEFAULT_COMPOSER_TOOLBAR];
      }
    },
  },
);

export const composerToolbarStore = store;

export function useComposerToolbar(): string[] {
  return useStore(store);
}

export function setComposerToolbar(ids: string[]): void {
  store.set(ids);
}
