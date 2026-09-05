import { currentProviderEntry } from "./providerCatalogRegistry";
import {
  providerSurfaceColor,
  providerSurfaceColors,
  type ProviderVisual,
} from "./visualHostMap.ts";

export type ThemeMode = "light" | "dark";
export type { ProviderVisual } from "./visualHostMap.ts";
export { applyVisualHostPlugins } from "./visualHostMap.ts";

/** Relative luminance of a #RRGGBB accent, or null when the token is not hex. */
export function providerAccentLuminance(accent: string): number | null {
  const match = /^#([0-9a-f]{6})$/i.exec(accent.trim());
  if (!match) return null;
  const channels = [0, 2, 4].map((offset) =>
    Number.parseInt(match[1]!.slice(offset, offset + 2), 16) / 255
  );
  return channels
    .map((channel) =>
      channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
    )
    .reduce(
      (sum, channel, index) => sum + channel * [0.2126, 0.7152, 0.0722][index]!,
      0,
    );
}

/** Keep monochrome Provider marks readable on both papers. Grok cream #E8E4DC
 *  vanishes on light Settings/transcript paper the same way #18181B vanished
 *  on dark paper. */
export function readableProviderAccent(
  accent: string,
  mode: ThemeMode,
  fallback: string,
): string {
  const luminance = providerAccentLuminance(accent);
  if (luminance === null) return accent;
  if (mode === "dark") return luminance < 0.25 ? fallback : accent;
  if (luminance <= 0.55) return accent;
  const authored = accent.trim().toLowerCase();
  for (const pair of Object.values(providerSurfaceColors())) {
    if (pair.dark.primary.toLowerCase() === authored) return pair.light.primary;
  }
  return fallback;
}

export function providerVisual(
  provider: string,
  mode: ThemeMode,
  providerVersion?: string,
  providerDigest?: string,
): ProviderVisual {
  const authored = providerSurfaceColor(
    provider,
    providerVersion,
    providerDigest,
  );
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
