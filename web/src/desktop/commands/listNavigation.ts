/** Visible direct-jump label for the first ten list rows (`1`…`9`, then `0`). */
export function listJumpKey(index: number): string | null {
  if (index < 0 || index >= 10) return null;
  return index === 9 ? "0" : String(index + 1);
}

/** Resolve the second key after a list-local `g` prefix. */
export function listJumpIndex(key: string, itemCount: number): number | null {
  if (itemCount <= 0) return null;
  if (key === "g") return 0;
  if (!/^[0-9]$/.test(key)) return null;
  const slot = key === "0" ? 9 : Number(key) - 1;
  return slot < itemCount ? slot : null;
}

export type PendingItemAction =
  | "default"
  | "return"
  | "schedule"
  | "move"
  | "remove";

/** Bare item-scoped actions shared by Queue and Draft rows. */
export function pendingItemActionKey(key: string): PendingItemAction | null {
  return ({
    s: "default",
    r: "return",
    t: "schedule",
    m: "move",
    x: "remove",
  } as Record<string, PendingItemAction>)[key.toLocaleLowerCase()] ?? null;
}
