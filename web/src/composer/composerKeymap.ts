import { defaultKeymap } from "@codemirror/commands";
import type { KeyBinding } from "@codemirror/view";

// Cowboy owns Mod+Enter (submit/advance), Mod+I (italic), and Mod+Shift+K
// (insert link). Leaving CM6's defaultKeymap bindings in place caused them to
// insert blank lines, select parent syntax, or delete the whole line whenever
// an editor surface did not intercept the chord first.
const BLOCKED_DEFAULT_KEYS = new Set(["Mod-Enter", "Mod-i", "Shift-Mod-k"]);

function isBlockedBinding(binding: KeyBinding): boolean {
  return (binding.key !== undefined && BLOCKED_DEFAULT_KEYS.has(binding.key)) ||
    (binding.mac !== undefined && BLOCKED_DEFAULT_KEYS.has(binding.mac)) ||
    (binding.linux !== undefined && BLOCKED_DEFAULT_KEYS.has(binding.linux));
}

export const composerDefaultKeymap: readonly KeyBinding[] = defaultKeymap
  .filter((binding) => !isBlockedBinding(binding));
