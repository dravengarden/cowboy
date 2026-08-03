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
  if (
    active.tagName !== "TEXTAREA" &&
    !active.isContentEditable &&
    active.closest(".cm-content") === null
  ) {
    return false;
  }
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
