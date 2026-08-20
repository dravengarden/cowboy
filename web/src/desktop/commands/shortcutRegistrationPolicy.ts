import { parseShortcut } from "./shortcut";

export interface RegisteredShortcut {
  id: string;
  shortcut?: string;
  sequence?: readonly string[];
  contexts?: readonly string[];
  regions?: readonly string[];
}

function canonicalShortcut(shortcut: string): string {
  const stroke = parseShortcut(shortcut);
  return [
    stroke.ctrl ? "ctrl" : null,
    stroke.alt ? "alt" : null,
    stroke.shift ? "shift" : null,
    stroke.mod ? "mod" : null,
    stroke.key === "esc" ? "escape" : stroke.key,
  ].filter(Boolean).join("+");
}

function hasIntersection(
  left: readonly string[] | undefined,
  right: readonly string[] | undefined,
): boolean {
  if (!left || !right) return true;
  return left.some((value) => right.includes(value));
}

function scopesOverlap(
  left: RegisteredShortcut,
  right: RegisteredShortcut,
): boolean {
  return hasIntersection(left.contexts, right.contexts) &&
    hasIntersection(left.regions, right.regions);
}

function isGlobal(command: RegisteredShortcut): boolean {
  return !command.contexts && !command.regions;
}

function isBareProductLetter(shortcut: string): boolean {
  const stroke = parseShortcut(shortcut);
  return !stroke.mod && !stroke.ctrl && !stroke.alt && !stroke.shift &&
    /^[a-z]$/i.test(stroke.key);
}

/**
 * Enforce the product-level shortcut rules that browser/OS inventories cannot:
 * global letters need a prefix, overlapping direct chords cannot shadow one
 * another, and a prefix continuation has exactly one stable meaning.
 */
export function shortcutRegistrationConflict(
  command: RegisteredShortcut,
  registered: Iterable<RegisteredShortcut>,
): string | null {
  if (
    command.shortcut && isGlobal(command) &&
    isBareProductLetter(command.shortcut)
  ) {
    return `${command.shortcut} is a forbidden global bare product letter`;
  }

  for (const existing of registered) {
    if (existing.id === command.id) continue;
    if (
      command.shortcut && existing.shortcut &&
      canonicalShortcut(command.shortcut) ===
        canonicalShortcut(existing.shortcut) &&
      scopesOverlap(command, existing)
    ) {
      return `${command.shortcut} overlaps ${existing.id}`;
    }
    if (
      command.sequence && existing.sequence &&
      command.sequence.length === existing.sequence.length &&
      command.sequence.every((stroke, index) =>
        canonicalShortcut(stroke) ===
          canonicalShortcut(existing.sequence?.[index] ?? "")
      )
    ) {
      return `${
        command.sequence.join(" then ")
      } already belongs to ${existing.id}`;
    }
  }
  return null;
}

export function assertShortcutRegistrationAllowed(
  command: RegisteredShortcut,
  registered: Iterable<RegisteredShortcut>,
): void {
  const conflict = shortcutRegistrationConflict(command, registered);
  if (conflict) {
    throw new Error(`Unsafe Desktop shortcut for ${command.id}: ${conflict}`);
  }
}
