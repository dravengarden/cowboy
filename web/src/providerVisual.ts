import { currentProviderEntry } from "./providerCatalogRegistry";

export type ThemeMode = "light" | "dark";

export interface ProviderVisual {
  primary: string;
  secondary: string;
}

/** Distinct, theme-readable accents. Catalog metadata can still supply a
 *  package-specific pair; these keep first-party Providers from collapsing
 *  into Cowboy purple or vanishing on dark paper (Grok shipped #18181B). */
export const PROVIDER_SURFACE_COLORS: Record<
  string,
  { readonly light: ProviderVisual; readonly dark: ProviderVisual }
> = {
  "claude-code": {
    light: { primary: "#C65D3A", secondary: "#9A4A30" },
    dark: { primary: "#E08A6A", secondary: "#D97757" },
  },
  grok: {
    light: { primary: "#44403C", secondary: "#78716C" },
    dark: { primary: "#E8E4DC", secondary: "#C4B5A0" },
  },
  gemini: {
    light: { primary: "#1A73E8", secondary: "#7C4DFF" },
    dark: { primary: "#8AB4F8", secondary: "#C58AF9" },
  },
  codex: {
    light: { primary: "#3B5BDB", secondary: "#0F766E" },
    dark: { primary: "#8EA2FF", secondary: "#2DD4BF" },
  },
  "claude-deepseek": {
    light: { primary: "#4F46E5", secondary: "#7C3AED" },
    dark: { primary: "#A5B4FC", secondary: "#C4B5FD" },
  },
  "codex-deepseek": {
    light: { primary: "#0369A1", secondary: "#0F766E" },
    dark: { primary: "#7DD3FC", secondary: "#5EEAD4" },
  },
};

export function providerVisual(
  provider: string,
  mode: ThemeMode,
  providerVersion?: string,
  providerDigest?: string,
): ProviderVisual {
  const authored = PROVIDER_SURFACE_COLORS[provider];
  if (authored) return authored[mode];
  const packaged = currentProviderEntry(
    provider,
    providerVersion,
    providerDigest,
  );
  if (packaged) {
    return {
      primary: packaged.manifest.display.accent,
      secondary: packaged.manifest.display.secondary_accent,
    };
  }
  const dark = mode === "dark";
  return {
    primary: dark ? "#A9B4C7" : "#52606D",
    secondary: dark ? "#D1D8E5" : "#7B8794",
  };
}
