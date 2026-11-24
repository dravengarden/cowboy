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
/** True when `target` sits on an interactive button/link/control that owns bare Enter. */
export function isInteractiveControl(target: EventTarget | null): boolean {
  return target instanceof Element &&
    target.closest(
      "button, a[href], [role='button'], [role='tab'], [role='menuitem'], summary",
    ) !== null;
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
