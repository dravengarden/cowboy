// While the leader waits for its next key, the focused text field is made
// non-editable (FOCUS.md "Leader").
//
// macOS hands each key to an active CJK input method before the page sees
// it. In an editable field the keydown then arrives as Process/229, and
// preventing it cannot stop the composition: `⌘K` `Q` from Insert typed a
// "q" into the Composer. A field that is not editable has no text-input
// client, so the input method leaves the continuation key alone and the
// page receives it as an ordinary key. The field keeps focus, caret and
// Vim mode; editability returns the moment the leader ends.

interface Lock {
  readonly element: HTMLElement;
  readonly restore: () => void;
}

let lock: Lock | null = null;

export function lockEditableForLeader(): void {
  if (lock) return;
  const element = document.activeElement;
  if (
    (element instanceof HTMLInputElement ||
      element instanceof HTMLTextAreaElement) && !element.readOnly &&
    !element.disabled
  ) {
    element.readOnly = true;
    lock = {
      element,
      restore: () => {
        element.readOnly = false;
      },
    };
    return;
  }
  // The editor's Vim Normal sink is not editable, but the DOM selection it
  // leaves behind still sits in the editor, and composition can follow the
  // selection there. Lock that editor too.
  const anchor = document.getSelection()?.anchorNode ?? null;
  const selectionHost = (anchor instanceof Element ? anchor : anchor?.parentElement)
    ?.closest<HTMLElement>("[contenteditable='true']") ?? null;
  const editable = element instanceof HTMLElement && element.isContentEditable
    ? element
    : selectionHost;
  if (editable) {
    const host = editable.closest<HTMLElement>("[contenteditable='true']") ??
      editable;
    // A tabindex keeps the host focusable once it stops being editable, so
    // the browser does not move focus (and Vim does not see a blur).
    const addedTabIndex = !host.hasAttribute("tabindex");
    if (addedTabIndex) host.tabIndex = -1;
    host.setAttribute("contenteditable", "false");
    lock = {
      element: host,
      restore: () => {
        host.setAttribute("contenteditable", "true");
        if (addedTabIndex) host.removeAttribute("tabindex");
      },
    };
  }
}

export function unlockEditableForLeader(): void {
  const current = lock;
  lock = null;
  current?.restore();
}

/** The field the leader locked, for tests and diagnostics. */
export function leaderLockedElement(): HTMLElement | null {
  return lock?.element ?? null;
}
