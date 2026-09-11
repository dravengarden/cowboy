import { appearanceStyle, appIcon } from "./appIcons";

function rgb(hex: string): number[] {
  return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
}
export function mixColor(a: string, b: string, weight: number): string {
  const other = rgb(b);
  return "#" +
    rgb(a).map((v, i) =>
      Math.round(v * (1 - weight) + other[i]! * weight).toString(16).padStart(
        2,
        "0",
      )
    ).join("");
}
function luminance(hex: string): number {
  const [r = 0, g = 0, b = 0] = rgb(hex).map((v) => {
    const s = v / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
export function colorContrast(a: string, b: string): number {
  const x = luminance(a), y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
function readable(seed: string, background: string, dark: boolean): string {
  for (let step = 0; step <= 100; step++) {
    const candidate = mixColor(seed, dark ? "#ffffff" : "#000000", step / 100);
    if (
      colorContrast(candidate, background) >= 4.6 &&
      colorContrast(candidate, dark ? "#101014" : "#ffffff") >= 4.6
    ) return candidate;
  }
  return dark ? "#ffffff" : "#000000";
}
function rgba(hex: string, alpha: number): string {
  return `rgba(${rgb(hex).join(", ")}, ${alpha})`;
}

export function appearancePalette(id: string, dark: boolean) {
  const icon = appIcon(id), style = appearanceStyle(id);
  // Style selection changes only the two accent families. Shared surfaces and
  // neutral states keep status colors visually consistent across all styles.
  const canvas = dark ? "#101014" : "#f7fdff";
  const paper = dark ? mixColor(canvas, "#ffffff", 0.055) : "#ffffff";
  const controlSurface = dark ? paper : canvas;
  const primary = readable(style.themeColor, controlSurface, dark);
  const secondary = readable(
    icon.brim === style.themeColor ? icon.crown : icon.brim,
    controlSurface,
    dark,
  );
  return {
    mode: dark ? "dark" as const : "light" as const,
    primary: {
      main: primary,
      light: mixColor(primary, "#ffffff", 0.16),
      dark: readable(mixColor(primary, "#000000", 0.16), controlSurface, dark),
      contrastText: dark ? "#101014" : "#ffffff",
    },
    secondary: {
      main: secondary,
      light: mixColor(secondary, "#ffffff", 0.16),
      dark: readable(
        mixColor(secondary, "#000000", 0.16),
        controlSurface,
        dark,
      ),
      contrastText: dark ? "#101014" : "#ffffff",
    },
    background: { default: canvas, paper },
    text: {
      primary: dark ? "#f5f5f8" : "#202027",
      secondary: dark ? "#bdbdc9" : "#565660",
    },
    divider: rgba(dark ? "#ffffff" : "#000000", 0.12),
    action: {
      hover: rgba(dark ? "#ffffff" : "#000000", dark ? 0.08 : 0.04),
      selected: rgba(dark ? "#ffffff" : "#000000", dark ? 0.16 : 0.08),
    },
  };
}
