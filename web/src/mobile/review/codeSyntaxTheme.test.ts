import { assert, assertNotEquals } from "jsr:@std/assert";
import { codeSyntaxPalette } from "./codeSyntaxTheme.ts";

function luminance(hex: string): number {
  const channels = [1, 3, 5].map((offset) => {
    const value = Number.parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045
      ? value / 12.92
      : ((value + 0.055) / 1.055) ** 2.4;
  });
  return channels[0]! * 0.2126 + channels[1]! * 0.7152 +
    channels[2]! * 0.0722;
}

function contrast(a: string, b: string): number {
  const high = Math.max(luminance(a), luminance(b));
  const low = Math.min(luminance(a), luminance(b));
  return (high + 0.05) / (low + 0.05);
}

Deno.test("code syntax palettes stay readable on Cowboy surfaces", () => {
  for (
    const [mode, background] of [
      ["light", "#f6f4fb"],
      ["dark", "#15111d"],
    ] as const
  ) {
    const palette = codeSyntaxPalette(mode);
    for (const [name, color] of Object.entries(palette)) {
      assert(
        contrast(color, background) >= 4.5,
        `${mode} ${name} must meet WCAG AA contrast`,
      );
    }
  }
});

Deno.test("light and dark code palettes are independently tuned", () => {
  const light = codeSyntaxPalette("light");
  const dark = codeSyntaxPalette("dark");
  for (const name of Object.keys(light) as (keyof typeof light)[]) {
    assertNotEquals(light[name], dark[name]);
  }
});
