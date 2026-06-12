// Which composer commands appear in the fullscreen markdown toolbar, and in what
// order — the persisted, user-curatable config the toolbar renders from (the
// Obsidian "Manage toolbar options" model). Per device (localStorage), same
// pattern as desktopLayout.ts / readingSettings.ts. The phase-2 settings sheet
// edits this list (add / remove / drag-reorder); until then it's the default.
import { persisted, useStore } from "./_store/mod.ts";
import { COMPOSER_COMMANDS_BY_ID } from "./composerCommands";

// Today's fullscreen toolbar actions + highlight + indent/outdent, in a flat
// Obsidian-like order (no dividers — the row scrolls horizontally).
export const DEFAULT_COMPOSER_TOOLBAR: readonly string[] = [
  "undo",
  "redo",
  "heading",
  "bold",
  "italic",
  "strikethrough",
  "highlight",
  "code",
  "link",
  "bulletList",
  "numberedList",
  "checklist",
  "quote",
  "codeBlock",
  "indent",
  "outdent",
  "mention",
  "slash",
  "attach",
  "paste",
];

const store = persisted<string[]>(
  "cowboy:composer-toolbar",
  [...DEFAULT_COMPOSER_TOOLBAR],
  {
    serialize: (v) => JSON.stringify(v),
    deserialize: (s) => {
      try {
        const v: unknown = JSON.parse(s);
        // Keep only ids that still resolve to a command (a removed/renamed
        // command in a stale stored list shouldn't render a blank button).
        return Array.isArray(v) && v.every((x) => typeof x === "string")
          ? (v as string[]).filter((id) => id in COMPOSER_COMMANDS_BY_ID)
          : [...DEFAULT_COMPOSER_TOOLBAR];
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
