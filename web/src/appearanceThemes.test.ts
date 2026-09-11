import { createTheme } from "@mui/material";
import { assert, assertEquals } from "jsr:@std/assert";
import {
  APP_ICONS,
  DEFAULT_APP_ICON,
  resolveIconPreference,
} from "./appIcons.ts";
import { appearancePalette, colorContrast } from "./appearanceThemes.ts";

Deno.test("all twenty styles retain readable light and dark controls and text", () => {
  for (const dark of [false, true]) {
    const accents = new Set<string>();
    for (const icon of APP_ICONS) {
      const p = appearancePalette(icon.id, dark);
      accents.add(p.primary.main);
      for (const background of [p.background.default, p.background.paper]) {
        for (
          const foreground of [
            p.text.primary,
            p.text.secondary,
            p.primary.main,
            p.secondary.main,
          ]
        ) {
          assert(
            colorContrast(foreground, background) >= 4.5,
            `${icon.id} ${dark} text ${foreground} on ${background}`,
          );
        }
      }
      for (const color of [p.primary, p.secondary]) {
        for (const background of [color.main, color.dark]) {
          assert(
            colorContrast(color.contrastText, background) >= 4.5,
            `${icon.id} ${dark} button contrast`,
          );
        }
      }
    }
    assertEquals(accents.size, 20);
  }
  assertEquals(
    appearancePalette(DEFAULT_APP_ICON, true).primary.main,
    "#51c9ff",
  );
  assertEquals(
    appearancePalette(DEFAULT_APP_ICON, true).background.default,
    "#101014",
  );
});

Deno.test("default migration preserves deliberate and archived custom styles", () => {
  assertEquals(resolveIconPreference(null, null), DEFAULT_APP_ICON);
  assertEquals(resolveIconPreference(null, "palette-054"), DEFAULT_APP_ICON);
  assertEquals(resolveIconPreference(null, "palette-126"), "palette-126");
  assertEquals(resolveIconPreference("palette-054", null), "palette-054");
  assertEquals(
    resolveIconPreference("default", "palette-126"),
    DEFAULT_APP_ICON,
  );
  assertEquals(resolveIconPreference("../../external", null), DEFAULT_APP_ICON);
});

Deno.test("style changes preserve surfaces and MUI semantic status colors", () => {
  for (const dark of [false, true]) {
    const baseline = createTheme({
      palette: { mode: dark ? "dark" : "light" },
    });
    const { primary: _primary, secondary: _secondary, ...shared } =
      appearancePalette(DEFAULT_APP_ICON, dark);
    for (const icon of APP_ICONS) {
      const palette = appearancePalette(icon.id, dark);
      const { primary: _p, secondary: _s, ...rest } = palette;
      assertEquals(rest, shared, `${icon.id}: only accents may vary`);
      const theme = createTheme({ palette });
      for (const status of ["error", "warning", "success", "info"] as const) {
        assertEquals(theme.palette[status], baseline.palette[status]);
      }
    }
  }
});
