/** Resolve the second key after a list-local `g` prefix. */
export function listJumpIndex(key: string, itemCount: number): number | null {
  if (itemCount <= 0) return null;
  if (key === "g") return 0;
  if (!/^[0-9]$/.test(key)) return null;
  const slot = key === "0" ? 9 : Number(key) - 1;
  return Math.min(itemCount - 1, slot);
}
