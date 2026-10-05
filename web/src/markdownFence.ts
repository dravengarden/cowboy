// Whether a fenced code block's source is still waiting for its closing fence.
// While a reply streams, Markdown renders an unclosed fence as a code block
// that keeps growing; its content is not final until the fence closes.

const OPENING_FENCE = /^ {0,3}(`{3,}|~{3,})/;
const CLOSING_FENCE = /^ {0,3}(`{3,}|~{3,})\s*$/;

/** `source` is the block's exact Markdown slice, from its opening fence. An
 *  indented (unfenced) block is never open. */
export function fencedCodeIsOpen(source: string): boolean {
  const lines = source.split("\n");
  const opening = OPENING_FENCE.exec(lines[0] ?? "")?.[1];
  if (!opening) return false;
  if (lines.length < 2) return true;
  const closing = CLOSING_FENCE.exec(lines[lines.length - 1] ?? "")?.[1];
  return !(closing && closing[0] === opening[0] && closing.length >= opening.length);
}
