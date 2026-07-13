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
  if (!match?.[1]) return event.key;
  const key = match[1].toLowerCase();
  return event.shiftKey ? key.toUpperCase() : key;
}
