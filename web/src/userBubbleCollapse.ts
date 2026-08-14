export const USER_BUBBLE_COLLAPSE_PX = 200;
export const USER_BUBBLE_COLLAPSE_BUFFER_PX = 80;

/** Clamp a user bubble only when there is enough overflow to justify Show more.
 * Unmeasured content still clamps so a long bubble cannot flash fully open. */
export function userBubbleShouldClamp(options: {
  measured: boolean;
  naturalHeight: number;
  expanded: boolean;
  containsImage: boolean;
}): boolean {
  if (options.expanded || options.containsImage) return false;
  if (!options.measured) return true;
  return options.naturalHeight >
    USER_BUBBLE_COLLAPSE_PX + USER_BUBBLE_COLLAPSE_BUFFER_PX;
}
