export function resolvePluginSlotComponent<T>(
  module: {
    default?: T;
    slots?: Partial<Record<string, T>>;
  },
  slot: string,
): T | null {
  if (module.slots) {
    const mapped = module.slots[slot];
    return typeof mapped === "function" ? mapped : null;
  }
  return typeof module.default === "function" ? module.default : null;
}
