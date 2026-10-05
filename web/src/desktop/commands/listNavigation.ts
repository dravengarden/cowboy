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
