import { isMac } from "../../platform";
import { getVimMode } from "../../vimModeStore";
import { getVimSetting } from "../../vimSetting";
import { DESKTOP_COMPOSER_FORMAT_CHORDS } from "./workspaceShortcuts";

/** The direct formatting chord of a composer command, if it has one.
 * Off macOS, Mod is Ctrl: plain Ctrl-B/Ctrl-I are Vim scroll/jump keys, so
 * outside Vim Insert they stay with the editor (toolbar and palette remain).
 * Shift/Alt chords have no Vim meaning and always format. */
export function formatChord(
  id: string,
): { shortcut: string; allowInEditor: () => boolean } | Record<string, never> {
  const shortcut = DESKTOP_COMPOSER_FORMAT_CHORDS[id];
  if (!shortcut) return {};
  return {
    shortcut,
    allowInEditor: () =>
      isMac || /\b(Shift|Alt)\+/.test(shortcut) || !getVimSetting() ||
      getVimMode() === "insert",
  };
}
