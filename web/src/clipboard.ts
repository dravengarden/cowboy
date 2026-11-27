// Clipboard API for secure production origins, with a legacy fallback for the
// plain-HTTP development bridge where navigator.clipboard is unavailable.
export async function copyText(text: string): Promise<boolean> {
  try {
    if (globalThis.navigator?.clipboard?.writeText) {
      await globalThis.navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // A denied or unfocused secure context can still use the fallback below.
  }
  try {
    const doc = globalThis.document;
    const textarea = doc.createElement("textarea");
    textarea.value = text;
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    doc.body.appendChild(textarea);
    textarea.select();
    // eslint-disable-next-line @typescript-eslint/no-deprecated -- HTTP dev fallback
    const copied = doc.execCommand("copy");
    textarea.remove();
    return copied;
  } catch {
    return false;
  }
}

/** Write both clean semantic HTML and readable plain text in one synchronous
 *  gesture frame (preserving iOS Safari / WKWebView activation). Falls back to
 *  plain-text copy when ClipboardItem or rich MIME writes are unavailable. */
export async function copyRichText(plain: string, html: string): Promise<boolean> {
  try {
    const clipboard = globalThis.navigator?.clipboard;
    const Item = globalThis.ClipboardItem;
    if (clipboard?.write && typeof Item === "function" && typeof Blob === "function") {
      const item = new Item({
        "text/plain": new Blob([plain], { type: "text/plain" }),
        "text/html": new Blob([html], { type: "text/html" }),
      });
      await clipboard.write([item]);
      return true;
    }
  } catch {
    // Fall back to plain-text clipboard write below.
  }
  return copyText(plain);
}
