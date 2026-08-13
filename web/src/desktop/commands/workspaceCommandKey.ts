/**
 * Physical-key identity for Desktop workspace Vim commands.
 *
 * macOS input sources can report `Process`, marked text, or a translated
 * character through `event.key`. Workspace navigation is positional Vim input,
 * not text, so use the physical letter key while retaining Shift for `G`.
 */
export function workspaceCommandKey(event: {
  code: string;
  key: string;
  shiftKey: boolean;
}): string {
  const match = /^Key([A-Z])$/.exec(event.code);
  if (match?.[1]) {
    const key = match[1].toLowerCase();
    return event.shiftKey ? key.toUpperCase() : key;
  }
  if (event.code === "Comma") return event.shiftKey ? "<" : ",";
  if (event.code === "Period") return event.shiftKey ? ">" : ".";
  const physical = ({
    BracketLeft: "[",
    BracketRight: "]",
    Enter: "Enter",
    Escape: "Escape",
    Backspace: "Backspace",
    Tab: "Tab",
  } as Readonly<Record<string, string>>)[event.code];
  return physical ?? event.key;
}
