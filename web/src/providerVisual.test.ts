import { assertEquals, assertNotEquals } from "jsr:@std/assert";
import { providerVisual } from "./providerVisual.ts";

Deno.test("catalog-unavailable Providers use theme-safe generic visuals", () => {
  const dark = providerVisual("future-agent", "dark");
  const light = providerVisual("future-agent", "light");
  assertEquals(dark.primary, "#A9B4C7");
  assertEquals(light.primary, "#52606D");
  assertNotEquals(dark.primary, dark.secondary);
  assertNotEquals(dark.primary, light.primary);
});
