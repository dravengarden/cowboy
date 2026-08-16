import { assertEquals, assertNotEquals } from "jsr:@std/assert";
import { PROVIDER_SURFACE_COLORS, providerVisual } from "./providerVisual.ts";

Deno.test("catalog-unavailable Providers use theme-safe generic visuals", () => {
  const dark = providerVisual("future-agent", "dark");
  const light = providerVisual("future-agent", "light");
  assertEquals(dark.primary, "#A9B4C7");
  assertEquals(light.primary, "#52606D");
  assertNotEquals(dark.primary, dark.secondary);
  assertNotEquals(dark.primary, light.primary);
});

Deno.test("first-party Providers keep distinct readable accents", () => {
  const ids = Object.keys(PROVIDER_SURFACE_COLORS);
  const darkPrimaries = new Set(
    ids.map((id) => providerVisual(id, "dark").primary),
  );
  const lightPrimaries = new Set(
    ids.map((id) => providerVisual(id, "light").primary),
  );
  assertEquals(darkPrimaries.size, ids.length);
  assertEquals(lightPrimaries.size, ids.length);
  assertNotEquals(providerVisual("grok", "dark").primary, "#18181B");
  assertEquals(providerVisual("grok", "dark").primary, "#E8E4DC");
  assertEquals(providerVisual("claude-code", "dark").primary, "#E08A6A");
});
