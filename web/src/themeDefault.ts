const THEME_STORAGE_KEY = "cowboy-theme-mode";
const SYSTEM_DEFAULT_MIGRATION_KEY = "cowboy:theme-system-default-v1";

type ThemeStorage = Pick<Storage, "getItem" | "setItem">;

// An earlier Cowboy release wrote `light` for every first visit. Removing that
// product default later only fixed fresh browsers: existing installs kept the
// seeded value and looked like an intentional Light choice forever. Reset that
// legacy seed once, before the shared theme hook reads it. A later user choice
// is preserved because the migration marker has already been written.
export function migrateThemeDefaultToSystem(
  storage?: ThemeStorage,
): void {
  try {
    const target = storage ?? globalThis.localStorage;
    if (target.getItem(SYSTEM_DEFAULT_MIGRATION_KEY) === "1") return;
    if (target.getItem(THEME_STORAGE_KEY) === "light") {
      target.setItem(THEME_STORAGE_KEY, "system");
    }
    target.setItem(SYSTEM_DEFAULT_MIGRATION_KEY, "1");
  } catch {
    // Private or locked-down WebViews may deny storage. The shared hook retains
    // its in-memory System fallback, so theme selection must still mount.
  }
}
