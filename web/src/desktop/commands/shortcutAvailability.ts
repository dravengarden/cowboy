export type ShortcutAvailability = "inactive" | "available" | "active";

/**
 * Resolve one shortcut slot without conflating context ownership with business
 * enablement. `active` is reserved for a pending prefix/mode or an open action;
 * a merely focused command remains available.
 */
export function shortcutAvailability(
  scopeAvailable: boolean,
  active = false,
): ShortcutAvailability {
  if (!scopeAvailable) return "inactive";
  return active ? "active" : "available";
}

/** Prefix and continuation slots intentionally have different armed states. */
export function sequentialShortcutAvailability({
  scopeAvailable,
  armed,
  prefix,
}: {
  scopeAvailable: boolean;
  armed: boolean;
  prefix: boolean;
}): ShortcutAvailability {
  if (!scopeAvailable) return "inactive";
  if (!armed) return prefix ? "available" : "inactive";
  return prefix ? "active" : "available";
}

/**
 * A leader slot (`␣B`) is one keycap for the whole sequence: available at rest
 * while its scope owns focus, lit (`active`) while the leader is armed so every
 * key that works next stands out at once.
 */
export function leaderShortcutAvailability(
  scopeAvailable: boolean,
  armed: boolean,
): ShortcutAvailability {
  if (!scopeAvailable) return "inactive";
  return armed ? "active" : "available";
}
