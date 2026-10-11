/** How long after the last keystroke a focused editor still speaks for someone
 * who is using it. */
export const EDITOR_ENGAGEMENT_MS = 60_000;

/** How long after the last keystroke the page is still mid-task wherever the
 * focus has moved since. */
export const RECENT_INPUT_MS = 20_000;

export interface EditorEngagement {
  /** The focused element accepts text. */
  readonly focusedEditable: boolean;
  /** This window has the system's keyboard focus. */
  readonly windowFocused: boolean;
  /** An IME composition is open. */
  readonly composing: boolean;
  /** Milliseconds since the last key or text input in an editable element. */
  readonly sinceInputMs: number;
}

/** Whether a focused editor should hold back a client update.
 *
 * Focus alone is not use. Desktop is keyboard-first, so the caret rests in the
 * composer for the whole session, including after the user has left the
 * window, and a gate on focus alone never opens there: a deployed build then
 * waits for a manual reload while Mobile, whose keyboard dismisses, has long
 * taken it. An open composition always holds; otherwise the editor holds only
 * while this window is focused and was typed in recently. Typing in the last
 * few seconds holds without the focus: the pointer moved the caret out, the
 * user did not stop working. */
export function editorHoldsUpdate(
  engagement: EditorEngagement,
  engagementMs = EDITOR_ENGAGEMENT_MS,
): boolean {
  if (engagement.composing && engagement.focusedEditable) return true;
  if (!engagement.windowFocused) return false;
  // Someone who typed a moment ago is mid-task even when the caret has since
  // left the editor: clicking another session to read something is part of
  // writing the prompt, and that click must not be what reloads the page.
  if (engagement.sinceInputMs < RECENT_INPUT_MS) return true;
  return engagement.focusedEditable && engagement.sinceInputMs < engagementMs;
}
