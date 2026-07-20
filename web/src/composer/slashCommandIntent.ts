// ACP exposes slash commands as text, so transport must preserve the difference
// between an explicitly selected completion and an ordinary leading path. A
// WORD JOINER is invisible and non-whitespace: agents read the literal text, but
// their start-of-prompt slash-command dispatchers no longer mistake it for a
// command. Keep this guard at the submission boundary, not in the editor doc.
export const LITERAL_SLASH_GUARD = "\u2060";

export function matchesSelectedSlashCommand(
  text: string,
  command: string | null,
): boolean {
  if (!command || !text.startsWith(`/${command}`)) return false;
  const rest = text.slice(command.length + 1);
  return rest.length === 0 || /^\s/u.test(rest);
}

export function prepareUserPrompt(
  text: string,
  selectedCommand: string | null,
): string {
  if (matchesSelectedSlashCommand(text, selectedCommand)) return text;
  const match = /^(\s*)\//u.exec(text);
  if (!match) return text;
  const prefix = match[1] ?? "";
  return `${prefix}${LITERAL_SLASH_GUARD}${text.slice(prefix.length)}`;
}
