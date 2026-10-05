import { isMac } from "../../platform";
import { getVimMode } from "../../vimModeStore";
import { getVimSetting } from "../../vimSetting";
import { DESKTOP_COMPOSER_FORMAT_CHORDS } from "./workspaceShortcuts";

/** Direct Mod+B / Mod+I formatting for a composer command, if it has one.
 * Off macOS, Mod is Ctrl: Ctrl-B/Ctrl-I are Vim scroll/jump keys, so outside
 * Vim Insert they stay with the editor and formatting uses the leader. */
export function formatChord(
  id: string,
): { shortcut: string; allowInEditor: () => boolean } | Record<string, never> {
  const shortcut = DESKTOP_COMPOSER_FORMAT_CHORDS[id];
  if (!shortcut) return {};
  return {
    shortcut,
    allowInEditor: () => isMac || !getVimSetting() || getVimMode() === "insert",
  };
}
