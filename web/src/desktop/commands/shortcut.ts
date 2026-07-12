export interface ShortcutStroke {
  key: string;
  mod: boolean;
  shift: boolean;
  alt: boolean;
}

export function parseShortcut(shortcut: string): ShortcutStroke {
  const parts = shortcut.split("+").map((part) => part.trim()).filter(Boolean);
  const key = parts.at(-1)?.toLowerCase() ?? "";
  return {
    key,
    mod: parts.slice(0, -1).some((part) => part.toLowerCase() === "mod"),
    shift: parts.slice(0, -1).some((part) => part.toLowerCase() === "shift"),
    alt: parts.slice(0, -1).some((part) => part.toLowerCase() === "alt"),
  };
}

export function matchesShortcut(
  stroke: ShortcutStroke,
  event: Pick<
    KeyboardEvent,
    "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"
  > & { code?: string },
  mac: boolean,
): boolean {
  const modDown = mac ? event.metaKey : event.ctrlKey;
  const otherModDown = mac ? event.ctrlKey : event.metaKey;
  // Number-row shortcuts are positional. Some macOS input sources report a
  // produced symbol (rather than "2") in `key` while Cmd is held, but `code`
  // remains Digit2. This keeps session slots stable across keyboard layouts.
  const keyMatches = event.key.toLowerCase() === stroke.key ||
    (/^\d$/.test(stroke.key) &&
      (event.code === `Digit${stroke.key}` || event.code === `Numpad${stroke.key}`));
  return keyMatches &&
    modDown === stroke.mod &&
    !otherModDown &&
    event.shiftKey === stroke.shift &&
    event.altKey === stroke.alt;
}

export function isTextEditingTarget(target: EventTarget | null): boolean {
  const element = target instanceof Element ? target : null;
  return element?.matches(
    "input, textarea, select, [contenteditable='true'], .cm-content",
  ) ?? false;
}
