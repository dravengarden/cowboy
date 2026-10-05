// The Desktop modal primitive (FOCUS.md "Modals"): every Desktop dialog is
// driven by the same Vim grammar, read from its DOM, so a new dialog needs no
// keyboard code of its own.
//
//   Insert  a text field owns every key; `Esc`/`Ctrl-[` leaves it for Normal
//           with the cursor on that field (a second `Esc` closes).
//   Normal  `J/K` row down/up, `H/L` left/right inside a row, or the top tabs
//           when the row has nothing beside it; `1`–`9` pick a tab, `[`/`]`
//           step tabs; `gg`/`G` first/last; `I`/`A`/`Enter` edit the field
//           under the cursor; `Enter` activates a button, `Esc` closes.
//
// Rows are read from geometry: controls whose vertical centres share a band
// form one row, so a tab strip, a button bar or a field with its trailing
// button is a row without markup. Regions with their own keymap (a tree, a
// listbox, an editor, `[data-desktop-keys='own']`) are one stop on the way
// and keep their keys once focused; a dialog containing
// `[data-desktop-modal-keys='own']` runs its whole keymap itself (Settings).

import { workspaceCommandKey } from "./workspaceCommandKey";

/** Regions inside a modal that keep their own keys while focused. */
export const DESKTOP_MODAL_OWN_KEYS = [
  "[role='tree']",
  "[role='treegrid']",
  "[role='grid']",
  "[role='listbox']",
  "[role='menu']",
  ".cm-editor",
  "[data-desktop-keys='own']",
  "[data-desktop-shortcut-scope='exclusive']",
].join(", ");

const CONTROLS = [
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

// A read-only input is a picker (WorkspacePicker), not a field: Normal
// keys reach it and Enter opens it.
const TEXT_FIELD = [
  "input:not([type='checkbox']):not([type='radio']):not([type='button'])" +
  ":not([type='submit']):not([type='reset']):not([type='range'])" +
  ":not([type='file']):not([type='color']):not([type='hidden'])" +
  ":not([readonly])",
  "textarea:not([readonly])",
  "[contenteditable='true']",
].join(", ");

/** The Normal-mode cursor on a text field: its outlined box, not the input. */
const FIELD_CURSOR = "data-desktop-field-cursor";

export function isModalTextField(element: Element | null): boolean {
  return element?.matches(TEXT_FIELD) ?? false;
}

function visible(element: HTMLElement): boolean {
  if (element.getClientRects().length === 0) return false;
  const style = getComputedStyle(element);
  return style.visibility !== "hidden" && style.display !== "none";
}

function enabled(element: HTMLElement): boolean {
  return !element.matches(":disabled, [aria-disabled='true']") &&
    element.closest("[aria-hidden='true'], [inert], [data-leader-ignore]") ===
      null;
}

/** The element that holds the Normal cursor for a text field. */
export function fieldCursorHost(field: HTMLElement): HTMLElement {
  return field.closest<HTMLElement>(".MuiFormControl-root, .MuiInputBase-root") ??
    field.parentElement ?? field;
}

/** One stop per control, top to bottom; an own-keys region is one stop. */
export function modalStops(root: HTMLElement): HTMLElement[] {
  const stops: HTMLElement[] = [];
  const regions = new Set<Element>();
  for (const element of root.querySelectorAll<HTMLElement>(CONTROLS)) {
    if (!visible(element) || !enabled(element)) continue;
    const region = element.closest(DESKTOP_MODAL_OWN_KEYS);
    if (region && region !== root && root.contains(region)) {
      if (regions.has(region)) continue;
      regions.add(region);
      // The region's roving entry, else its first control.
      stops.push(
        region.querySelector<HTMLElement>("[tabindex='0']") ?? element,
      );
      continue;
    }
    // A native input inside a button-like wrapper is one target.
    const owner = element.parentElement?.closest<HTMLElement>(CONTROLS);
    if (owner && root.contains(owner) && !owner.matches("input, textarea")) {
      continue;
    }
    stops.push(element);
  }
  return stops;
}

export interface ModalBox {
  readonly top: number;
  readonly bottom: number;
  readonly left: number;
  readonly right: number;
}

/** Group stops into rows: a stop joins the row its vertical centre falls in. */
export function modalRows<T>(
  stops: readonly T[],
  box: (stop: T) => ModalBox,
): T[][] {
  const sorted = [...stops].sort((a, b) => {
    const left = box(a);
    const right = box(b);
    return (left.top + left.bottom) - (right.top + right.bottom) ||
      left.left - right.left;
  });
  const rows: { band: ModalBox; stops: T[] }[] = [];
  for (const stop of sorted) {
    const rect = box(stop);
    const centre = (rect.top + rect.bottom) / 2;
    const row = rows.find(({ band }) =>
      centre >= band.top && centre <= band.bottom
    );
    if (row) row.stops.push(stop);
    else rows.push({ band: rect, stops: [stop] });
  }
  return rows.map((row) =>
    row.stops.sort((a, b) => box(a).left - box(b).left)
  );
}

function rectOf(element: HTMLElement): ModalBox {
  const target = isModalTextField(element) ? fieldCursorHost(element) : element;
  return target.getBoundingClientRect();
}

/** The first tablist of the dialog: its tabs answer digits, `[`/`]` and
 *  `H/L` from a row with nothing beside it. */
function modalTabs(root: HTMLElement): HTMLElement[] {
  const list = root.querySelector<HTMLElement>("[role='tablist']");
  if (!list) return [];
  return [...list.querySelectorAll<HTMLElement>("[role='tab']")]
    .filter((tab) => visible(tab) && enabled(tab));
}

/** Put the Normal cursor on a stop: a text field shows it on its box. */
export function focusModalStop(stop: HTMLElement): void {
  if (isModalTextField(stop)) {
    const host = fieldCursorHost(stop);
    if (!host.hasAttribute("tabindex")) host.tabIndex = -1;
    host.setAttribute(FIELD_CURSOR, "");
    host.focus({ preventScroll: true });
    host.scrollIntoView({ block: "nearest" });
    return;
  }
  stop.focus({ preventScroll: true });
  stop.scrollIntoView({ block: "nearest" });
}

/** Leave a text field for Normal with the cursor on it. */
export function enterModalNormal(field: HTMLElement): void {
  focusModalStop(field);
}

/** Select a tab the way its own keyboard grammar would, keeping the
 *  cursor where it is unless it was on the tabs. */
export function selectModalTab(tab: HTMLElement, cursor: Element | null): void {
  const onTabs = cursor?.closest("[role='tablist']") !== null &&
    cursor?.closest("[role='tablist']") === tab.closest("[role='tablist']");
  const handled = !tab.dispatchEvent(
    new CustomEvent(DESKTOP_TAB_SELECT_EVENT, {
      bubbles: true,
      cancelable: true,
      detail: { focus: onTabs },
    }),
  );
  if (!handled) {
    tab.click();
    if (onTabs) tab.focus({ preventScroll: true });
  }
  if (!onTabs && cursor instanceof HTMLElement) {
    requestAnimationFrame(() => {
      if (cursor.isConnected) cursor.focus({ preventScroll: true });
    });
  }
}

/** Dispatched on a tab to select it without a pointer activation; a tablist
 *  that handles it cancels it (SegmentedTabs). `detail.focus` asks it to
 *  move DOM focus to the tab. */
export const DESKTOP_TAB_SELECT_EVENT = "cowboy:desktop-tab-select";

let goPending = false;
let goTimer: ReturnType<typeof setTimeout> | undefined;

/**
 * Run one key of the modal grammar on the topmost dialog. Returns true when
 * the key was consumed. IME composition and the dialog leader are resolved
 * by the caller first.
 */
export function handleDesktopModalKey(
  event: KeyboardEvent,
  modal: HTMLElement,
): boolean {
  if (!modal.matches("[role='dialog']")) return false;
  if (modal.querySelector("[data-desktop-modal-keys='own']")) return false;
  const target = event.target instanceof HTMLElement ? event.target : null;
  if (target && !modal.contains(target)) return false;
  if (target?.closest(DESKTOP_MODAL_OWN_KEYS)) return false;
  const consume = (): true => {
    event.preventDefault();
    event.stopPropagation();
    return true;
  };
  // Insert: the field keeps every key except the one that leaves it.
  if (target && isModalTextField(target)) {
    const escape = (event.key === "Escape" && !event.ctrlKey &&
      !event.metaKey && !event.altKey && !event.shiftKey) ||
      (event.ctrlKey && !event.metaKey && !event.altKey &&
        event.code === "BracketLeft");
    if (!escape) return false;
    enterModalNormal(target);
    return consume();
  }
  if (event.metaKey || event.ctrlKey || event.altKey || event.repeat) {
    return false;
  }
  const key = workspaceCommandKey(event);
  const field = target?.hasAttribute(FIELD_CURSOR)
    ? target.querySelector<HTMLElement>(TEXT_FIELD)
    : null;
  if (field && (key === "i" || key === "a" || key === "Enter")) {
    field.focus({ preventScroll: true });
    if (
      key === "a" &&
      (field instanceof HTMLInputElement ||
        field instanceof HTMLTextAreaElement)
    ) {
      const end = field.value.length;
      field.setSelectionRange(end, end);
    }
    return consume();
  }
  if (
    key === "Enter" && target?.matches(
      "[role='checkbox'], [role='switch'], [role='radio'], input[type='checkbox'], input[type='radio']",
    )
  ) {
    target.click();
    return consume();
  }
  const tabs = modalTabs(modal);
  const selectedTab = tabs.findIndex((tab) =>
    tab.getAttribute("aria-selected") === "true"
  );
  const stepTab = (delta: number): boolean => {
    if (tabs.length < 2) return false;
    const from = selectedTab < 0 ? 0 : selectedTab;
    selectModalTab(
      tabs[(from + delta + tabs.length) % tabs.length]!,
      target,
    );
    return true;
  };
  const digit = /^(?:Digit|Numpad)([1-9])$/.exec(event.code);
  if (digit && !event.shiftKey) {
    const tab = tabs[Number(digit[1]) - 1];
    if (!tab) return false;
    selectModalTab(tab, target);
    return consume();
  }
  if (key === "[" || key === "]") {
    return stepTab(key === "]" ? 1 : -1) ? consume() : false;
  }
  if (!["h", "j", "k", "l", "g", "G"].includes(key)) {
    goPending = false;
    return false;
  }
  const stops = modalStops(modal);
  if (stops.length === 0) return false;
  const current = field ?? stops.find((stop) =>
    stop === target || (target !== null && stop.contains(target))
  ) ?? null;
  if (key === "G" || (key === "g" && goPending)) {
    goPending = false;
    focusModalStop(key === "G" ? stops.at(-1)! : stops[0]!);
    return consume();
  }
  if (key === "g") {
    goPending = true;
    globalThis.clearTimeout(goTimer);
    goTimer = globalThis.setTimeout(() => {
      goPending = false;
    }, 1000);
    return consume();
  }
  goPending = false;
  const rows = modalRows(stops, rectOf);
  const rowIndex = current
    ? rows.findIndex((row) => row.includes(current))
    : -1;
  if (key === "j" || key === "k") {
    const next = rowIndex < 0
      ? (key === "j" ? 0 : rows.length - 1)
      : rowIndex + (key === "j" ? 1 : -1);
    const row = rows[Math.max(0, Math.min(rows.length - 1, next))]!;
    // Land on the selected tab of a tab row, else the stop nearest in x.
    const x = current
      ? (rectOf(current).left + rectOf(current).right) / 2
      : 0;
    const stop = row.find((candidate) =>
      candidate.getAttribute("aria-selected") === "true"
    ) ?? row.reduce((best, candidate) => {
      const centre = (box: ModalBox) => (box.left + box.right) / 2;
      return Math.abs(centre(rectOf(candidate)) - x) <
          Math.abs(centre(rectOf(best)) - x)
        ? candidate
        : best;
    });
    focusModalStop(stop);
    return consume();
  }
  // h / l
  const delta = key === "l" ? 1 : -1;
  if (current?.matches("[role='tab']")) {
    return stepTab(delta) ? consume() : false;
  }
  const row = rowIndex >= 0 ? rows[rowIndex]! : [];
  if (row.length > 1 && current) {
    const index = row.indexOf(current);
    const next = row[Math.max(0, Math.min(row.length - 1, index + delta))]!;
    if (next !== current) focusModalStop(next);
    return consume();
  }
  return stepTab(delta) ? consume() : false;
}
