// Monotonic ownership epoch so a slower rejected clipboard write can never
// overwrite a newer copy attempt with its plain-text fallback.
let clipboardEpoch = 0;

export function claimClipboardEpoch(): number {
  clipboardEpoch += 1;
  return clipboardEpoch;
}

export function isCurrentClipboardEpoch(epoch: number): boolean {
  return epoch === clipboardEpoch;
}

// Clipboard API for secure production origins, with a legacy fallback for the
// plain-HTTP development bridge where navigator.clipboard is unavailable.
export async function copyText(text: string, epoch?: number): Promise<boolean> {
  const ownEpoch = epoch ?? claimClipboardEpoch();
  if (!isCurrentClipboardEpoch(ownEpoch)) return false;
  try {
    if (globalThis.navigator?.clipboard?.writeText) {
      await globalThis.navigator.clipboard.writeText(text);
      return isCurrentClipboardEpoch(ownEpoch);
    }
  } catch {
    // A denied or unfocused secure context can still use the fallback below.
  }
  if (!isCurrentClipboardEpoch(ownEpoch)) return false;
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
    return copied && isCurrentClipboardEpoch(ownEpoch);
  } catch {
    return false;
  }
}

export interface RichCopyResult {
  readonly ok: boolean;
  /** True when dual-MIME write threw and the post-await plain fallback also
   *  failed (e.g. iOS user-activation expired), so a fresh tap should retry
   *  synchronously with plain text. */
  readonly retryPlainText: boolean;
  /** True when a newer copy attempt superseded this write before completion. */
  readonly superseded?: boolean;
}

function supportsRichClipboardWrite(): boolean {
  const clipboard = globalThis.navigator?.clipboard;
  const Item = globalThis.ClipboardItem;
  if (typeof clipboard?.write !== "function" || typeof Item !== "function" || typeof Blob !== "function") {
    return false;
  }
  const supports = (Item as unknown as { supports?: (type: string) => boolean }).supports;
  if (typeof supports === "function") {
    return supports("text/html") && supports("text/plain");
  }
  return true;
}

/** Write both clean semantic HTML and readable plain text in one synchronous
 *  gesture frame (preserving iOS Safari / WKWebView activation). Pre-checks
 *  ClipboardItem.supports synchronously before calling clipboard.write so
 *  unsupported engines fall straight into synchronous plain-text copy without
 *  spending the user gesture on a doomed async write. */
export async function copyRichTextDetailed(
  plain: string,
  html: string,
  epoch?: number,
): Promise<RichCopyResult> {
  const ownEpoch = epoch ?? claimClipboardEpoch();
  if (!isCurrentClipboardEpoch(ownEpoch)) {
    return { ok: false, retryPlainText: false, superseded: true };
  }
  if (!supportsRichClipboardWrite()) {
    const ok = await copyText(plain, ownEpoch);
    if (!isCurrentClipboardEpoch(ownEpoch)) {
      return { ok: false, retryPlainText: false, superseded: true };
    }
    return { ok, retryPlainText: false };
  }
  try {
    const item = new globalThis.ClipboardItem({
      "text/plain": new Blob([plain], { type: "text/plain" }),
      "text/html": new Blob([html], { type: "text/html" }),
    });
    await globalThis.navigator.clipboard.write([item]);
    if (!isCurrentClipboardEpoch(ownEpoch)) {
      return { ok: false, retryPlainText: false, superseded: true };
    }
    return { ok: true, retryPlainText: false };
  } catch {
    if (!isCurrentClipboardEpoch(ownEpoch)) {
      return { ok: false, retryPlainText: false, superseded: true };
    }
    const ok = await copyText(plain, ownEpoch);
    if (!isCurrentClipboardEpoch(ownEpoch)) {
      return { ok: false, retryPlainText: false, superseded: true };
    }
    return { ok, retryPlainText: !ok };
  }
}

export async function copyRichText(plain: string, html: string): Promise<boolean> {
  return (await copyRichTextDetailed(plain, html)).ok;
}
