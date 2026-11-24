import { isMac } from "../../platform";
import { desktopImeOwnsKey } from "./imeShortcut";
import { matchesShortcut, parseShortcut } from "./shortcut";
import {
  DESKTOP_NEW_SESSION_DRAFT_EVENT,
  DESKTOP_PENDING_ROW_EDIT_EVENT,
} from "../promptBuffer";

const MOD_ENTER_STROKE = parseShortcut("Mod+Enter");

export type PendingItemAction =
  | "return"
  | "schedule"
  | "move"
  | "document"
  | "remove";

/** Bare item-scoped actions shared by Queue and Draft rows.
 *  Primary send/deliver moved to `Mod+Enter` so bare `s` never fires a send. */
export function pendingItemActionKey(key: string): PendingItemAction | null {
  return ({
    r: "return",
    t: "schedule",
    m: "move",
    d: "document",
    x: "remove",
  } as Record<string, PendingItemAction>)[key.toLocaleLowerCase()] ?? null;
}

export function isPendingModEnter(event: KeyboardEvent): boolean {
  if (desktopImeOwnsKey(event)) return false;
  return matchesShortcut(MOD_ENTER_STROKE, event, isMac);
}

function triggerPendingEdit(
  item: HTMLElement | undefined,
  entryMode: "insert" | "normal",
): boolean {
  if (!item) return false;
  const handled = !item.dispatchEvent(
    new CustomEvent(DESKTOP_PENDING_ROW_EDIT_EVENT, {
      bubbles: true,
      cancelable: true,
      detail: { entryMode },
    }),
  );
  if (handled) return true;
  const fallback = item.querySelector<HTMLElement>(
    "[data-desktop-item-action='edit'], button[aria-label='Edit']",
  );
  if (!fallback) return false;
  fallback.click();
  return true;
}

/** Handle Queue/Draft list-specific keys (reorder, Esc/h back to canvas,
 *  i/l/Enter unified edit, n new draft, Mod+Enter deliver, item verbs). */
export function handlePendingListKey({
  event,
  key,
  region,
  items,
  active,
  reordering,
  focusComposer,
}: {
  event: KeyboardEvent;
  key: string;
  region: HTMLElement;
  items: readonly HTMLElement[];
  active: number;
  reordering: boolean;
  focusComposer: () => void;
}): boolean {
  if (isPendingModEnter(event)) {
    if (active < 0) return false;
    const action = items[active]?.querySelector<HTMLElement>(
      "[data-desktop-item-action='default']",
    );
    if (!action) return false;
    event.preventDefault();
    event.stopPropagation();
    action.click();
    return true;
  }
  if (event.ctrlKey || event.metaKey || event.altKey) return false;
  if (key === "Escape" || (!reordering && key === "h")) {
    event.preventDefault();
    event.stopPropagation();
    if (reordering) {
      region.querySelector<HTMLElement>("[data-desktop-pending-list]")?.dispatchEvent(
        new CustomEvent("cowboy:desktop-release-reorder"),
      );
    } else {
      focusComposer();
    }
    return true;
  }
  if (
    !reordering &&
    key.toLowerCase() === "n" &&
    region.dataset.desktopRegion === "prompt.draft" &&
    !event.repeat
  ) {
    event.preventDefault();
    event.stopPropagation();
    globalThis.dispatchEvent(new CustomEvent(DESKTOP_NEW_SESSION_DRAFT_EVENT));
    return true;
  }
  if (items.length === 0) return false;
  if (key.toLowerCase() === "o" && !event.repeat) {
    event.preventDefault();
    event.stopPropagation();
    region.querySelector<HTMLElement>("[data-desktop-pending-list]")?.dispatchEvent(
      new CustomEvent("cowboy:desktop-toggle-reorder"),
    );
    items[Math.max(0, active)]?.focus({ preventScroll: true });
    return true;
  }
  if (reordering && (key === "j" || key === "k")) {
    event.preventDefault();
    event.stopPropagation();
    items[Math.max(0, active)]?.dispatchEvent(new CustomEvent("cowboy:desktop-reorder", {
      bubbles: true,
      detail: { delta: key === "j" ? 1 : -1 },
    }));
    return true;
  }
  if (!reordering && active >= 0) {
    if (key.toLowerCase() === "i" && !event.repeat) {
      if (triggerPendingEdit(items[active], "insert")) {
        event.preventDefault();
        event.stopPropagation();
        return true;
      }
    }
    if ((key === "Enter" || key.toLowerCase() === "l") && !event.repeat) {
      if (triggerPendingEdit(items[active], "normal")) {
        event.preventDefault();
        event.stopPropagation();
        return true;
      }
    }
    const pendingAction = pendingItemActionKey(key);
    const action = pendingAction
      ? items[active]?.querySelector<HTMLElement>(
        `[data-desktop-item-action='${pendingAction}']`,
      )
      : null;
    if (action) {
      event.preventDefault();
      event.stopPropagation();
      action.click();
      return true;
    }
  }
  return false;
}

/** The nearest scrolling ancestor of a list item. */
export function listScroller(item: HTMLElement): HTMLElement | null {
  let element = item.parentElement;
  while (element && element !== document.body) {
    const overflow = getComputedStyle(element).overflowY;
    if (
      (overflow === "auto" || overflow === "scroll") &&
      element.scrollHeight > element.clientHeight
    ) return element;
    element = element.parentElement;
  }
  return null;
}

export type ListViewKey =
  | "half-down"
  | "half-up"
  | "page-down"
  | "page-up";

/** Vim's Ctrl-D/U/F/B in a list, by physical key. */
export function listViewKey(
  event: { code: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean },
): ListViewKey | null {
  if (!event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return null;
  return ({
    KeyD: "half-down",
    KeyU: "half-up",
    KeyF: "page-down",
    KeyB: "page-up",
  } as Record<string, ListViewKey>)[event.code] ?? null;
}

/**
 * The row a page motion lands on: as many rows as fit in the given share
 * of the viewport, from the current row, clamped to the list.
 */
export function listPageTarget(
  active: number,
  count: number,
  visibleRows: number,
  motion: ListViewKey,
): number {
  const share = motion === "half-down" || motion === "half-up" ? 0.5 : 1;
  const step = Math.max(1, Math.floor(visibleRows * share));
  const direction = motion === "half-down" || motion === "page-down" ? 1 : -1;
  return Math.max(0, Math.min(count - 1, Math.max(0, active) + direction * step));
}

/** scrollTop that puts a row at the top, centre or bottom (zt/zz/zb). */
export function listScrollFor(
  where: "top" | "center" | "bottom",
  row: { top: number; height: number },
  viewport: { scrollTop: number; height: number },
): number {
  const offset = where === "top"
    ? row.top
    : where === "center"
    ? row.top + row.height / 2 - viewport.height / 2
    : row.top + row.height - viewport.height;
  return Math.max(0, viewport.scrollTop + offset);
}
