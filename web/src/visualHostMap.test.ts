import { assert, assertEquals } from "jsr:@std/assert";
import {
  applyVisualHostPlugins,
  BUNDLED_PROVIDER_SURFACE_COLORS,
  providerSurfaceColor,
} from "./visualHostMap.ts";

Deno.test("first-party Provider colors are generated from bundled host.json", () => {
  const source = Deno.readTextFileSync(
    new URL("./visualHostMap.ts", import.meta.url),
  );
  assert(source.includes("bundledHostPlugins"));
  assertEquals(source.includes("#E08A6A"), false);
  assertEquals(source.includes("#E8E4DC"), false);
  assertEquals(
    BUNDLED_PROVIDER_SURFACE_COLORS.grok?.dark.primary,
    "#E8E4DC",
  );
  assertEquals(
    BUNDLED_PROVIDER_SURFACE_COLORS["claude-code"]?.dark.primary,
    "#E08A6A",
  );
  assertEquals(providerSurfaceColor("grok")?.dark.primary, "#E8E4DC");
  assertEquals(providerSurfaceColor("claude-code")?.dark.primary, "#E08A6A");
});

Deno.test("activated host plugins overlay Provider surface colors", () => {
  try {
    applyVisualHostPlugins([{
      id: "grok",
      visual: {
        light: { primary: "#111111", secondary: "#222222" },
        dark: { primary: "#EEEEEE", secondary: "#DDDDDD" },
      },
    }]);
    assertEquals(providerSurfaceColor("grok")?.dark.primary, "#EEEEEE");
    assertEquals(providerSurfaceColor("claude-code")?.dark.primary, "#E08A6A");
  } finally {
    applyVisualHostPlugins([]);
  }
  assertEquals(providerSurfaceColor("grok")?.dark.primary, "#E8E4DC");
});
