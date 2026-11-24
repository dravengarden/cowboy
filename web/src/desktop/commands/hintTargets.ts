import { DESKTOP_JUMP_LABELS } from "./workspaceShortcuts";

/**
 * Hint labels (FOCUS.md "Labels"): the one way a key reaches a visible target
 * that has no fixed shortcut. List `f` labels rows; the leader inside a modal
 * labels every control of that modal. Labels exist only while armed.
 */
export interface DesktopHint {
  readonly label: string;
  readonly element: HTMLElement;
  readonly name: string;
  /** `row`: badge inside the left edge; `control`: badge on the top-right. */
  readonly placement: "row" | "control";
}

const MODAL_CONTROLS = [
  "button",
  "a[href]",
  "[role='button']",
  "[role='tab']",
  "[role='switch']",
  "[role='checkbox']",
  "[role='radio']",
  "[role='combobox']",
  "input:not([type='hidden'])",
  "textarea",
  "[contenteditable='true']",
].join(", ");

const COMPOSITE_LIST_WIDGET = "[role='tree'], [role='treegrid'], [role='listbox']";
const CLIPPING_OVERFLOW = /(auto|scroll|clip|hidden)/;

function visible(element: HTMLElement): boolean {
  if (element.getClientRects().length === 0) return false;
  const style = getComputedStyle(element);
  return style.visibility !== "hidden" && style.display !== "none";
}

/** True when `element` is completely scrolled or clipped outside the viewport
 *  or any scrollable/clipping ancestor up to `stopAt`. */
export function isClippedOutsideScrollContainer(
  element: HTMLElement,
  stopAt?: HTMLElement | null,
): boolean {
  const rect = element.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) return true;
  if (
    typeof globalThis.innerWidth === "number" && globalThis.innerWidth > 0 &&
    typeof globalThis.innerHeight === "number" && globalThis.innerHeight > 0
  ) {
    if (
      rect.bottom <= 0 || rect.top >= globalThis.innerHeight ||
      rect.right <= 0 || rect.left >= globalThis.innerWidth
    ) return true;
  }
  let ancestor = element.parentElement;
  while (ancestor) {
    const style = getComputedStyle(ancestor);
    const clipY = CLIPPING_OVERFLOW.test(style.overflowY);
    const clipX = CLIPPING_OVERFLOW.test(style.overflowX);
    if (clipY || clipX) {
      const box = ancestor.getBoundingClientRect();
      if (box.width > 0 && box.height > 0) {
        if (clipY && (rect.bottom <= box.top + 2 || rect.top >= box.bottom - 2)) {
          return true;
        }
        if (clipX && (rect.right <= box.left + 2 || rect.left >= box.right - 2)) {
          return true;
        }
      }
    }
    if (ancestor === stopAt || ancestor === document.body) break;
    ancestor = ancestor.parentElement;
  }
  return false;
}

function disabled(element: HTMLElement): boolean {
  return element.matches(":disabled, [aria-disabled='true']") ||
    element.closest("[aria-hidden='true'], [inert], [data-leader-ignore]") !==
      null;
}

/** Accessible name used both for the which-key entry and its mnemonic. */
export function hintName(element: HTMLElement): string {
  const labelled = element.getAttribute("aria-labelledby");
  const fromLabelledBy = labelled
    ? labelled.split(/\s+/).map((id) =>
      document.getElementById(id)?.textContent ?? ""
    ).join(" ")
    : "";
  const fieldLabel = element instanceof HTMLInputElement ||
      element instanceof HTMLTextAreaElement
    ? [...(element.labels ?? [])].map((label) => label.textContent ?? "")
      .join(" ") || element.placeholder
    : "";
  return (element.getAttribute("aria-label") || fromLabelledBy ||
    fieldLabel || visibleText(element) || element.title || "")
    .replace(/\s+/g, " ").trim();
}

/** Text a reader would hear: keycaps and other aria-hidden decoration (the
 *  `Esc` inside "Cancel") are not part of the control's name. */
function visibleText(element: HTMLElement): string {
  const clone = element.cloneNode(true) as HTMLElement;
  for (const hidden of clone.querySelectorAll("kbd, [aria-hidden='true']")) {
    hidden.remove();
  }
  return clone.textContent ?? "";
}

/**
 * Assign stable mnemonic letters: an explicit `data-leader-key` first, then
 * the first free letter of each word, then any free letter of the name, then
 * whatever is left. Same names always give the same keys.
 */
export function assignMnemonics(
  entries: readonly { name: string; preferred?: string | null }[],
  reserved: Iterable<string> = [],
): (string | null)[] {
  const pool = `${DESKTOP_JUMP_LABELS}1234567890`;
  const taken = new Set([...reserved].map((key) => key.toLowerCase()));
  const result: (string | null)[] = entries.map(() => null);
  const take = (index: number, key: string): boolean => {
    const lower = key.toLowerCase();
    if (!pool.includes(lower) || taken.has(lower)) return false;
    taken.add(lower);
    result[index] = lower;
    return true;
  };
  entries.forEach((entry, index) => {
    if (entry.preferred) take(index, entry.preferred);
  });
  const passes: ((name: string) => string[])[] = [
    (name) => name.split(/[^\p{L}\p{N}]+/u).map((word) => word[0] ?? ""),
    (name) => [...name],
  ];
  for (const pass of passes) {
    entries.forEach((entry, index) => {
      if (result[index] !== null) return;
      for (const candidate of pass(entry.name.toLowerCase())) {
        if (take(index, candidate)) return;
      }
    });
  }
  entries.forEach((_, index) => {
    if (result[index] !== null) return;
    for (const candidate of pool) {
      if (take(index, candidate)) return;
    }
  });
  return result;
}

/** Every visible operable control of the topmost modal, labelled. Off-screen
 *  scrolled controls and repeated rows inside tree/listbox widgets are skipped
 *  so the 36-letter pool stays reserved for reachable dialog controls. */
export function modalHints(root: HTMLElement): DesktopHint[] {
  const seen = new Set<HTMLElement>();
  const compositeRegions = new Set<Element>();
  const controls = [...root.querySelectorAll<HTMLElement>(MODAL_CONTROLS)]
    .flatMap((element): HTMLElement[] => {
      if (!visible(element) || disabled(element)) return [];
      if (isClippedOutsideScrollContainer(element, root)) return [];
      const region = element.closest(COMPOSITE_LIST_WIDGET);
      if (region && region !== root && root.contains(region)) {
        if (compositeRegions.has(region)) return [];
        compositeRegions.add(region);
        const entry = region.querySelector<HTMLElement>("[tabindex='0']");
        const pick = entry && visible(entry) && !disabled(entry) &&
            !isClippedOutsideScrollContainer(entry, root)
          ? entry
          : element;
        if (seen.has(pick)) return [];
        seen.add(pick);
        return [pick];
      }
      // A native input inside a labelled button-like wrapper is one target.
      const owner = element.parentElement?.closest<HTMLElement>(MODAL_CONTROLS);
      if (owner && root.contains(owner) && !owner.matches("input, textarea")) {
        return [];
      }
      if (seen.has(element)) return [];
      seen.add(element);
      return [element];
    })
    .sort((left, right) => {
      const a = left.getBoundingClientRect();
      const b = right.getBoundingClientRect();
      return a.top - b.top || a.left - b.left;
    });
  const named = controls.map((element) => ({
    element,
    name: hintName(element),
    preferred: element.dataset.leaderKey ?? null,
  })).filter((entry) => entry.name.length > 0);
  const keys = assignMnemonics(named);
  return named.flatMap((entry, index) => {
    const label = keys[index];
    return label
      ? [{ label, element: entry.element, name: entry.name, placement: "control" }]
      : [];
  });
}

/** Letter labels for a list's visible items, top to bottom. */
export function listHints(items: readonly HTMLElement[]): DesktopHint[] {
  return items.slice(0, DESKTOP_JUMP_LABELS.length).map((element, index) => ({
    label: DESKTOP_JUMP_LABELS[index]!,
    element,
    name: hintName(element),
    placement: "row",
  }));
}

/** True when a previously labelled hint target is still connected, visible,
 *  enabled, unclipped in its scroll chain, and owned by the current topmost
 *  modal (with no popup above it). */
export function isHintActionable(
  element: HTMLElement,
  modal?: HTMLElement | null,
): boolean {
  if (!element.isConnected || !visible(element) || disabled(element)) return false;
  if (modal !== undefined) {
    if (!modal || popupOwnsKeys() || topmostModal() !== modal || !modal.contains(element)) {
      return false;
    }
  }
  return !isClippedOutsideScrollContainer(element, modal ?? undefined);
}

/** Activate a labelled target the way a pointer would. */
export function activateHint(element: HTMLElement): void {
  element.focus({ preventScroll: false });
  if (
    element.matches(
      "input:not([type='checkbox']):not([type='radio']), textarea, [contenteditable='true']",
    )
  ) return;
  if (element.matches("[role='combobox']:not(input)")) {
    // MUI Select opens from a primary mousedown, not click.
    element.dispatchEvent(
      new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }),
    );
    return;
  }
  element.click();
}

/** The topmost open modal dialog, if any (menus/popovers are not modals). */
export function topmostModal(): HTMLElement | null {
  const dialogs = [
    ...document.querySelectorAll<HTMLElement>(
      "[role='dialog'][aria-modal='true'], [data-desktop-shortcut-scope='exclusive']",
    ),
  ].filter(visible);
  return dialogs.at(-1) ?? null;
}

/** A menu, listbox or popover owns its own keys (type-ahead, arrows). */
export function popupOwnsKeys(): boolean {
  return document.querySelector(
    // Inline lists (Recent, directory defaults) are regions of the dialog.
    // Their native keys are scoped by modalNavigation to focus inside the list;
    // they must not disable Insert/Normal handling in the adjacent search.
    ".MuiPopover-root, .MuiAutocomplete-popper, [role='menu']:not([role='dialog'] [role='menu']), [role='listbox']:not([role='dialog'] [role='listbox'])",
  ) !== null;
}
