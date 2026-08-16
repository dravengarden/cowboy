/**
 * End the active Mobile composer editing session without touching unrelated
 * inputs. iOS keeps the software keyboard open while the focused native
 * textarea/contenteditable survives, so delivery and full-cover configuration
 * actions must explicitly release that one focus owner.
 */
export function releaseMobileComposerFocus(): boolean {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement)) return false;
  if (active.closest("[data-mobile-focus-composer='true']") === null) {
    return false;
  }

  // Opening the full-cover toolbar settings is a transition out of the whole
  // composer focus region, not merely out of its text node. By click time iOS
  // may already have moved document.activeElement from the editor to the Tune
  // button. Blur whichever focus owner remains so the sheet starts from a clean
  // boundary. The card itself promotes only from editor-area focus; utility
  // focus must never leave behind an expanded inert canvas.
  active.blur();
  return true;
}

/** End any text-entry focus owned by a confirmed Mobile delivery flow. */
export function dismissMobileSoftwareKeyboard(): boolean {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement)) return false;
  if (
    active.tagName !== "INPUT" &&
    active.tagName !== "TEXTAREA" &&
    !active.isContentEditable &&
    active.closest(".cm-content") === null
  ) {
    return false;
  }
  active.blur();
  return true;
}

/**
 * Distinguish a real software-keyboard dismissal from the short interval
 * between editor focus and iOS publishing its first visualViewport resize.
 */
export function didMobileSoftwareKeyboardClose(
  wasOpen: boolean,
  isOpen: boolean,
): boolean {
  return wasOpen && !isOpen;
}

/**
 * Keep a pending-message editor mounted through WebKit's keyboard-geometry
 * settle window. iOS can briefly publish a closed visual viewport while it is
 * promoting a long press into the native Paste/Select menu; unmounting the
 * textarea in that frame cancels the menu.
 */
export const mobilePendingKeyboardCloseSettleMs = 550;

/** A context reset blocks stale WebKit focus restoration from re-promoting UI. */
export function shouldPresentMobileKeyboardSurface(
  keyboardOpen: boolean,
  resetBlocked: boolean,
): boolean {
  return keyboardOpen && !resetBlocked;
}

/** Compact ↔ fullscreen remounts the editor. A one-frame visualViewport
 *  close must not blur the surviving first-responder. */
let mobileEditorFocusTransferUntil = 0;

export function beginMobileEditorFocusTransfer(): void {
  mobileEditorFocusTransferUntil = Date.now() + 800;
}

export function isMobileEditorFocusTransferPending(): boolean {
  return Date.now() < mobileEditorFocusTransferUntil;
}
