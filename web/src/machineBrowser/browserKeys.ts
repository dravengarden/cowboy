/** Browser commands for the remote Chromium, sent as the standard key chords
 * through the attached RFB session: there is no protocol for history,
 * reload or paste, so the viewer presses the keys a person would. */
import { KEYSYM } from "./touchKeyboard";

export type BrowserChord = "back" | "forward" | "reload" | "paste" | "address" | "enter";

/** One key transition: `down` presses, otherwise releases. */
export interface KeyStroke {
  keysym: number;
  code: string;
  down: boolean;
}

const ALT_L = { keysym: 0xffe9, code: "AltLeft" };
const CONTROL_L = { keysym: 0xffe3, code: "ControlLeft" };
const V = { keysym: 0x76, code: "KeyV" };
const L = { keysym: 0x6c, code: "KeyL" };
const RETURN = { keysym: KEYSYM.Return, code: "Enter" };
const F5 = { keysym: 0xffc2, code: "F5" };
const LEFT = { keysym: KEYSYM.Left, code: "ArrowLeft" };
const RIGHT = { keysym: KEYSYM.Right, code: "ArrowRight" };

/** Modifiers first, then the key (Chromium on Linux: Alt+Left, Alt+Right,
 * F5, Ctrl+V, Ctrl+L focusing and selecting the address bar, Return). */
const CHORDS: Record<BrowserChord, readonly { keysym: number; code: string }[]> = {
  back: [ALT_L, LEFT],
  forward: [ALT_L, RIGHT],
  reload: [F5],
  paste: [CONTROL_L, V],
  address: [CONTROL_L, L],
  enter: [RETURN],
};

/** The key transitions of a chord: every key pressed in order, then
 * released in reverse, so no modifier stays held on the remote. */
export function chordStrokes(chord: BrowserChord): KeyStroke[] {
  const keys = CHORDS[chord];
  return [
    ...keys.map((key) => ({ ...key, down: true })),
    ...[...keys].reverse().map((key) => ({ ...key, down: false })),
  ];
}
