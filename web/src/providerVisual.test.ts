import { assertEquals, assertNotEquals } from "jsr:@std/assert";
import { providerVisual } from "./providerVisual.ts";
import { applyVisualHostPlugins } from "./visualHostMap.ts";

Deno.test("catalog-unavailable Providers use theme-safe generic visuals", () => {
  const dark = providerVisual("future-agent", "dark");
  const light = providerVisual("future-agent", "light");
  assertEquals(dark.primary, "#A9B4C7");
  assertEquals(light.primary, "#52606D");
  assertNotEquals(dark.primary, dark.secondary);
  assertNotEquals(dark.primary, light.primary);
});

Deno.test("runtime host Providers keep distinct readable accents", () => {
  const hosts = [
    {
      id: "future-a",
      visual: {
        light: { primary: "#111111", secondary: "#222222" },
        dark: { primary: "#EEEEEE", secondary: "#DDDDDD" },
      },
    },
    {
      id: "future-b",
      visual: {
        light: { primary: "#333333", secondary: "#444444" },
        dark: { primary: "#CCCCCC", secondary: "#BBBBBB" },
      },
    },
  ];
  applyVisualHostPlugins(hosts);
  const ids = hosts.map((host) => host.id);
  const darkPrimaries = new Set(
    ids.map((id) => providerVisual(id, "dark").primary),
  );
  const lightPrimaries = new Set(
    ids.map((id) => providerVisual(id, "light").primary),
  );
  assertEquals(darkPrimaries.size, ids.length);
  assertEquals(lightPrimaries.size, ids.length);
  assertEquals(providerVisual("future-a", "dark").primary, "#EEEEEE");
  applyVisualHostPlugins([]);
});
