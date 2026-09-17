// Explicit actions that rewrite the editable under live IME marked text.
//
// iOS WebKit cannot survive a programmatic rewrite of the node that holds a
// composition: a `textarea.value` write, `setRangeText`, or a CM6 transaction
// that re-renders `.cm-content` strands the keyboard's composition and every
// later key is swallowed until the field is refocused (iOS 26 Simulator,
// PITFALLS #109/#110). Obsidian never edits the document while composing;
// Cowboy's Send, Clear all, dock Paste and image delete can arrive mid-pinyin.
//
// Blurring the editable is the one clean exit: WebKit commits the marked text
// as typed, refocuses the element to insert it, and fires `compositionend`.
// Only then is a rewrite safe. The commit is asynchronous, so the write runs
// from that event (after the editor's own handlers) or after a short fallback
// when no event arrives.

import { isAppleTouchDevice } from "../keyboardGeometry";

export const NATIVE_COMPOSITION_END_FALLBACK_MS = 250;

export function afterNativeCompositionEnds(
  editable: HTMLElement,
  then: () => void,
  fallbackMs = NATIVE_COMPOSITION_END_FALLBACK_MS,
): void {
  // `blur()` on an element that does not own focus is a no-op, so nothing
  // would ever end the composition; waiting would only delay the rewrite
  // without protecting it. (WKWebView can park `activeElement` on BODY under
  // a live composition, pitfall #45.)
  if (editable.ownerDocument.activeElement !== editable) {
    then();
    return;
  }
  let done = false;
  let timer = 0;
  const finish = (): void => {
    if (done) return;
    done = true;
    editable.removeEventListener("compositionend", onEnd, true);
    if (timer !== 0) globalThis.clearTimeout(timer);
    // WebKit refocuses the editable only transiently to insert the committed
    // text and may complete the blur afterwards, which would drop the
    // keyboard. This blur was part of the caller's explicit action, so the
    // same action restores the focus it took.
    if (editable.ownerDocument.activeElement !== editable) editable.focus();
    then();
  };
  const onEnd = (): void => {
    // Let the editor's own compositionend handling settle first.
    queueMicrotask(finish);
  };
  editable.addEventListener("compositionend", onEnd, true);
  timer = globalThis.setTimeout(finish, fallbackMs);
  editable.blur();
}

/**
 * Run `then` now, or after the live composition has been committed. Only
 * Apple touch WebKit needs the commit: Chrome Android finishes a Gboard
 * composition itself when the DOM is rewritten, and blurring there would
 * collapse the keyboard. `then(deferred)` tells the caller whether the
 * editable state may have changed since the call.
 */
export function withoutNativeComposition(
  editable: HTMLElement,
  composing: boolean,
  then: (deferred: boolean) => void,
): void {
  if (!composing || !isAppleTouchDevice(globalThis.navigator ?? {})) {
    then(false);
    return;
  }
  afterNativeCompositionEnds(editable, () => then(true));
}
