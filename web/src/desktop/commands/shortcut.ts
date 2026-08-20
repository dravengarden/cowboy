export interface ShortcutStroke {
  key: string;
  mod: boolean;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
}

export function parseShortcut(shortcut: string): ShortcutStroke {
  const parts = shortcut.split("+").map((part) => part.trim()).filter(Boolean);
  const key = parts.at(-1)?.toLowerCase() ?? "";
  return {
    key,
    mod: parts.slice(0, -1).some((part) => part.toLowerCase() === "mod"),
    ctrl: parts.slice(0, -1).some((part) => part.toLowerCase() === "ctrl"),
    shift: parts.slice(0, -1).some((part) => part.toLowerCase() === "shift"),
    alt: parts.slice(0, -1).some((part) => part.toLowerCase() === "alt"),
  };
}

export function matchesShortcut(
  stroke: ShortcutStroke,
  event:
    & Pick<
      KeyboardEvent,
      "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"
    >
    & { code?: string },
  mac: boolean,
  physicalBare = false,
): boolean {
  const expectedMeta = mac && stroke.mod;
  const expectedCtrl = stroke.ctrl || (!mac && stroke.mod);
  // Number-row shortcuts are positional. Some macOS input sources report a
  // produced symbol (rather than "2") in `key` while Cmd is held, but `code`
  // remains Digit2. This keeps session slots stable across keyboard layouts.
  const physicalCode = stroke.key.length === 1
    ? /^[a-z]$/.test(stroke.key)
      ? `Key${stroke.key.toUpperCase()}`
      : ({
        "/": "Slash",
        ".": "Period",
        ",": "Comma",
        "[": "BracketLeft",
        "]": "BracketRight",
        "\\": "Backslash",
      } as Record<string, string>)[stroke.key]
    : stroke.key === "enter"
    ? "Enter"
    : undefined;
  const keyMatches = event.key.toLowerCase() === stroke.key ||
    ((stroke.mod || stroke.ctrl || stroke.alt || physicalBare) &&
      physicalCode !== undefined &&
      event.code === physicalCode) ||
    (/^\d$/.test(stroke.key) &&
      (event.code === `Digit${stroke.key}` ||
        event.code === `Numpad${stroke.key}`));
  const producedBareSymbol = !stroke.mod && !stroke.ctrl && !stroke.alt &&
    !stroke.shift && [":", "?"].includes(stroke.key) &&
    event.key.toLowerCase() === stroke.key;
  return keyMatches &&
    event.metaKey === expectedMeta &&
    event.ctrlKey === expectedCtrl &&
    (productLetterIgnoresShift(stroke) || producedBareSymbol ||
      event.shiftKey === stroke.shift) &&
    event.altKey === stroke.alt;
}

/// Bare product letters (`F`, `Z`, `V`) treat Shift/Caps as case, not a
/// modifier. Vim regions keep `g`/`G` outside this matcher. Modified chords
/// (`Mod+Enter`, `Shift+J`) still require an exact Shift state.
export function productLetterIgnoresShift(stroke: ShortcutStroke): boolean {
  return !stroke.shift && !stroke.mod && !stroke.ctrl && !stroke.alt &&
    stroke.key.length === 1 && /[a-z]/.test(stroke.key);
}

export function isTextEditingTarget(target: EventTarget | null): boolean {
  const element = target instanceof Element ? target : null;
  return element?.matches(
    "input, textarea, select, [contenteditable='true'], .cm-content, [data-vim-command-sink]",
  ) ?? false;
}
