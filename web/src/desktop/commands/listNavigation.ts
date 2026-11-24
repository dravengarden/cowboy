export type PendingItemAction =
  | "default"
  | "return"
  | "schedule"
  | "move"
  | "document"
  | "remove";

/** Bare item-scoped actions shared by Queue and Draft rows. */
export function pendingItemActionKey(key: string): PendingItemAction | null {
  return ({
    s: "default",
    r: "return",
    t: "schedule",
    m: "move",
    d: "document",
    x: "remove",
  } as Record<string, PendingItemAction>)[key.toLocaleLowerCase()] ?? null;
}

/** Handle Queue/Draft list-specific keys (reorder, Esc/h back to canvas, item verbs). */
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
  if (key.toLowerCase() === "o" && !event.repeat) {
    event.preventDefault();
    event.stopPropagation();
    region.querySelector<HTMLElement>("[data-desktop-pending-list]")?.dispatchEvent(
      new CustomEvent("cowboy:desktop-toggle-reorder"),
    );
    items[Math.max(0, active)]?.focus({ preventScroll: true });
    return true;
  }
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
  if (reordering && (key === "j" || key === "k")) {
    event.preventDefault();
    event.stopPropagation();
    items[Math.max(0, active)]?.dispatchEvent(new CustomEvent("cowboy:desktop-reorder", {
      bubbles: true,
      detail: { delta: key === "j" ? 1 : -1 },
    }));
    return true;
  }
  if (active >= 0) {
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
