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
