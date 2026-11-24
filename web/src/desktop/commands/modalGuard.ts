import { isInputVimNormal } from "../vim/inputVim";
import { desktopImeOwnsKey } from "./imeShortcut";
import { machineBrowserOwnsKeys } from "./desktopShortcutScope";
import { popupOwnsKeys } from "./hintTargets";

const DIALOG_SELECTOR = "[role='dialog'][aria-modal='true']";

function dialogVisible(element: HTMLElement): boolean {
  if (typeof element.getClientRects === "function" && element.getClientRects().length === 0) {
    return false;
  }
  if (typeof getComputedStyle === "function") {
    const style = getComputedStyle(element);
    if (style.visibility === "hidden" || style.display === "none") return false;
  }
  return true;
}

/** Visible modal dialogs in DOM paint order (topmost last). */
export function visibleDialogs(root: Document = document): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(DIALOG_SELECTOR)].filter(dialogVisible);
}

/** The topmost visible modal dialog root, ignoring inner exclusive panels. */
export function topmostDialog(root: Document = document): HTMLElement | null {
  return visibleDialogs(root).at(-1) ?? null;
}

/** The modal dialog root that owns `node`. */
export function owningDialog(node: Element | null): HTMLElement | null {
  return node?.closest<HTMLElement>(DIALOG_SELECTOR) ?? null;
}

/**
 * True iff `node` belongs to the topmost visible modal dialog. Comparing the
 * enclosing `[role='dialog'][aria-modal='true']` avoids false mismatches when
 * an inner panel marks `data-desktop-shortcut-scope="exclusive"`.
 */
export function isTopmostDialog(
  node: Element | null,
  root: Document = document,
): boolean {
  const own = owningDialog(node);
  if (!own) return false;
  const top = topmostDialog(root);
  return top !== null && own === top;
}

const NON_TEXT_INPUT_TYPES = new Set([
  "button",
  "checkbox",
  "color",
  "file",
  "hidden",
  "radio",
  "range",
  "reset",
  "submit",
]);

/**
 * True when `target` is actively accepting text entry (never while `inputVim`
 * holds a native field in read-only Normal mode).
 */
export function isModalEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (isInputVimNormal(target)) return false;
  if (target instanceof HTMLTextAreaElement) return !target.readOnly;
  if (target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return !NON_TEXT_INPUT_TYPES.has(target.type) && !target.readOnly;
  }
  return target.matches("[contenteditable='true']") ||
    target.closest("[contenteditable='true']") !== null;
}

/**
 * Shared guard for component-owned modal keydown listeners:
 * stand aside during IME composition, behind the Machine browser, behind a
 * nested modal dialog, behind an open popup/menu, or inside editable text.
 */
const TABBABLE_SELECTOR = [
  "a[href]",
  "button",
  "input:not([type='hidden'])",
  "select",
  "textarea",
  "[tabindex]:not([tabindex='-1'])",
  "[contenteditable='true']",
].join(", ");

/** True when `target` sits on a native or ARIA interactive control that owns
 *  its own Tab/Enter/arrow keys (buttons, links, inputs, comboboxes, etc.). */
export function isInteractiveControl(target: EventTarget | null): boolean {
  return target instanceof Element &&
    target.closest(
      "button, a[href], input, select, textarea, [contenteditable='true'], [role='button'], [role='tab'], [role='menuitem'], [role='combobox'], [role='checkbox'], [role='radio'], [role='switch'], [role='slider'], [role='spinbutton'], [role='option'], [role='textbox'], summary",
    ) !== null;
}

/** Move keyboard focus cleanly past `region` in document order when a custom
 *  visual-order Tab walk (e.g. a column-reverse transcript) reaches its end. */
export function focusAdjacentOutsideRegion(
  region: HTMLElement,
  direction: -1 | 1,
): boolean {
  if (typeof document === "undefined") return false;
  const candidates = [...document.querySelectorAll<HTMLElement>(TABBABLE_SELECTOR)]
    .filter((element) => {
      if (region.contains(element)) return false;
      if (element.matches(":disabled, [aria-disabled='true']")) return false;
      if (element.closest("[aria-hidden='true'], [inert]")) return false;
      if (typeof element.getClientRects === "function" && element.getClientRects().length === 0) {
        return false;
      }
      const pos = region.compareDocumentPosition(element);
      return direction === 1
        ? Boolean(pos & Node.DOCUMENT_POSITION_FOLLOWING)
        : Boolean(pos & Node.DOCUMENT_POSITION_PRECEDING);
    });
  const target = direction === 1 ? candidates[0] : candidates.at(-1);
  if (!target) return false;
  target.focus({ preventScroll: false });
  return true;
}

export function modalHandlerShouldSkip(
  event: KeyboardEvent,
  ownNode: Element | null,
  {
    allowEditable = false,
    allowPopup = false,
    root = document,
  }: {
    allowEditable?: boolean;
    allowPopup?: boolean;
    root?: Document;
  } = {},
): boolean {
  if (desktopImeOwnsKey(event)) return true;
  const insideBrowser = ownNode?.closest("[data-machine-browser-modal]") !== null;
  if (!insideBrowser && machineBrowserOwnsKeys(event, root)) return true;
  if (ownNode !== null && !isTopmostDialog(ownNode, root)) return true;
  if (!allowPopup && popupOwnsKeys()) return true;
  if (!allowEditable && isModalEditableTarget(event.target)) return true;
  return false;
}
