import { currentProviderEntry } from "./providerCatalogRegistry";

export type ThemeMode = "light" | "dark";

export interface ProviderVisual {
  primary: string;
  secondary: string;
}

// Signed Provider display metadata is the only identity-specific color source.
export function providerVisual(
  provider: string,
  mode: ThemeMode,
  providerVersion?: string,
  providerDigest?: string,
): ProviderVisual {
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
