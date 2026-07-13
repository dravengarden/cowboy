const CODE_KEYS: Readonly<Record<string, string>> = {
  Backquote: "`",
  Backslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
  Comma: ",",
  Digit0: "0",
  Digit1: "1",
  Digit2: "2",
  Digit3: "3",
  Digit4: "4",
  Digit5: "5",
  Digit6: "6",
  Digit7: "7",
  Digit8: "8",
  Digit9: "9",
  Equal: "=",
  Minus: "-",
  Period: ".",
  Quote: "'",
  Semicolon: ";",
  Slash: "/",
};

const SPECIAL_KEYS: Readonly<Record<string, string>> = {
  ArrowDown: "<Down>",
  ArrowLeft: "<Left>",
  ArrowRight: "<Right>",
  ArrowUp: "<Up>",
  Backspace: "<BS>",
  Delete: "<Del>",
  End: "<End>",
  Enter: "<CR>",
  Escape: "<Esc>",
  Home: "<Home>",
  Insert: "<Ins>",
  PageDown: "<PageDown>",
  PageUp: "<PageUp>",
  Space: "<Space>",
};

const SHIFTED_KEYS: Readonly<Record<string, string>> = {
  "`": "~",
  "1": "!",
  "2": "@",
  "3": "#",
  "4": "$",
  "5": "%",
  "6": "^",
  "7": "&",
  "8": "*",
  "9": "(",
  "0": ")",
  "-": "_",
  "=": "+",
  "[": "{",
  "]": "}",
  "\\": "|",
  ";": ":",
  "'": "\"",
  ",": "<",
  ".": ">",
  "/": "?",
};

/** Convert a physical Desktop key into codemirror-vim's token vocabulary.
 *
 * `event.code` is deliberate: with a CJK input source active, `event.key` may
 * be `Process`, `Unidentified`, or marked text. Normal and Visual commands must
 * continue to mean the physical Vim key and must never enter the IME pipeline.
 */
export function vimCommandKey(event: KeyboardEvent): string | null {
  if (event.metaKey || event.altKey) return null;

  if (event.ctrlKey) {
    if (event.code === "BracketLeft") return "<C-[>";
    if (event.code === "Escape") return "<C-Esc>";
    if (event.code === "Space") return "<C-Space>";
    if (event.code === "Backspace") return "<C-BS>";
    if (/^Key[A-Z]$/.test(event.code)) {
      return `<C-${event.code.slice(3).toLowerCase()}>`;
    }
    return null;
  }

  // codemirror-vim gives these shifted physical keys distinct Normal-mode
  // meanings (`w`/`b`). Handle them before the unmodified special-key table.
  if (event.shiftKey && event.code === "Space") return "<S-Space>";
  if (event.shiftKey && event.code === "Backspace") return "<S-BS>";
  // A terminal sends Tab as Ctrl-I; preserve Vim's jumplist-forward behavior
  // instead of letting the hidden command sink leak browser focus.
  if (!event.shiftKey && event.code === "Tab") return "<C-i>";

  const special = SPECIAL_KEYS[event.code];
  if (special) return special;
  if (/^Key[A-Z]$/.test(event.code)) {
    const letter = event.code.slice(3).toLowerCase();
    return event.shiftKey ? letter.toUpperCase() : letter;
  }
  const key = CODE_KEYS[event.code];
  if (!key) return null;
  return event.shiftKey ? SHIFTED_KEYS[key] ?? key : key;
}
