// Clipboard decisions shared by the CM6 and native-textarea composers, aligned
// with Obsidian 1.13's paste pipeline (ClipboardManager.handleDataTransfer →
// tryPasteUrl → insertFiles → plain text).

/** Documents only contain "\n"; CM6 splits CRLF itself, a raw offset does not. */
export function normalizeClipboardText(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}

/** Whether a `text/html` payload is only the markup for one picture. */
export function htmlIsOnlyImage(html: string): boolean {
  const body = html
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/<\/?(?:html|head|body)\b[^>]*>/gi, "")
    .replace(/<meta\b[^>]*>/gi, "")
    .trim();
  return /^<img\s[^>]+>$/i.test(body);
}

/**
 * Obsidian lets rich text win over clipboard files unless its HTML is just
 * the copied picture. Office and spreadsheet apps put a rendered PNG beside
 * the table's HTML and plain text; attaching that PNG loses the text. Cowboy
 * inserts the plain-text representation (it does not convert HTML).
 */
export function pastedTextBeatsFiles(
  clipboard: Pick<DataTransfer, "getData">,
): boolean {
  const html = clipboard.getData("text/html");
  return html.trim() !== "" && clipboard.getData("text/plain") !== "" &&
    !htmlIsOnlyImage(html);
}

function parsesAsUrl(text: string): boolean {
  if (text === "" || text.includes(" ")) return false;
  try {
    new URL(text);
    return true;
  } catch {
    return false;
  }
}

/**
 * Obsidian's tryPasteUrl: a URL pasted over a single-line selection becomes
 * `[selection](url)`. Returns the replacement, or null for an ordinary paste.
 */
export function markdownLinkForPastedUrl(
  selectedText: string,
  pasted: string,
): string | null {
  if (selectedText === "" || selectedText.includes("\n")) return null;
  if (pasted.includes("\n") || !parsesAsUrl(pasted)) return null;
  return `[${selectedText}](${pasted})`;
}
