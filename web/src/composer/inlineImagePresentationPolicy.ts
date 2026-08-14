/**
 * Keep the proven CM6 block replacement on every surface. An attempted touch
 * variant nested the replacement in an ordinary line, but physical iPhone
 * acceptance showed that it regressed the native image-paste transaction.
 */
export function inlineImageUsesBlockDecoration(_touchInput: boolean): boolean {
  return true;
}
