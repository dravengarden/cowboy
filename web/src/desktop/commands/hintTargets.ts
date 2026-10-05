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

function visible(element: HTMLElement): boolean {
  if (element.getClientRects().length === 0) return false;
  const style = getComputedStyle(element);
  return style.visibility !== "hidden" && style.display !== "none";
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
    fieldLabel || element.textContent || element.title || "")
    .replace(/\s+/g, " ").trim();
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

/** Every operable control of the topmost modal, labelled. */
export function modalHints(root: HTMLElement): DesktopHint[] {
  const seen = new Set<HTMLElement>();
  const controls = [...root.querySelectorAll<HTMLElement>(MODAL_CONTROLS)]
    .filter((element) => {
      if (!visible(element) || disabled(element)) return false;
      // A native input inside a labelled button-like wrapper is one target.
      const owner = element.parentElement?.closest<HTMLElement>(MODAL_CONTROLS);
      if (owner && root.contains(owner) && !owner.matches("input, textarea")) {
        return false;
      }
      if (seen.has(element)) return false;
      seen.add(element);
      return true;
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
    ".MuiPopover-root, .MuiAutocomplete-popper, [role='menu'], [role='listbox']",
  ) !== null;
}
