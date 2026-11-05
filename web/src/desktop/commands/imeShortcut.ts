import { desktopKeyIntent } from "./keyIntent";

/** Keep Desktop commands layout-independent without stealing native text input.
 *  A composition, or an IME marker in a text field, owns the key; dialog
 *  buttons, lists and other chrome cannot hold marked text, so an idle CJK
 *  source labelling their keys `Process`/229 must not disable a shortcut. */
export function desktopImeOwnsKey(event: KeyboardEvent): boolean {
  return desktopKeyIntent(event).owner === "ime";
}

/** The key a component-local shortcut handler reads: `null` while an IME owns
 *  it, otherwise the physical identity (`j`, `G`, `Enter`, `ArrowDown`, ` `),
 *  so a CJK input source cannot turn `j` into `Process` or a candidate. */
export function shortcutKey(event: KeyboardEvent): string | null {
  const intent = desktopKeyIntent(event);
  return intent.owner === "ime" ? null : intent.key;
}
