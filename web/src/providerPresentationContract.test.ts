import { assertEquals } from "jsr:@std/assert";

const managementSource = await Deno.readTextFile(
  new URL("./ProviderManagement.tsx", import.meta.url),
);
const surfaceSource = await Deno.readTextFile(
  new URL("./ProviderSurface.tsx", import.meta.url),
);

Deno.test("Provider lifecycle cards stay collapsed behind a compact summary", () => {
  assertEquals(managementSource.includes("const [detailsOpen"), true);
  assertEquals(managementSource.includes("hidden={!detailsOpen}"), true);
  assertEquals(
    managementSource.includes('display: detailsOpen ? "grid" : "none"'),
    true,
  );
  assertEquals(managementSource.includes("borderTop:"), false);
});

Deno.test("Provider activity renderer consumes only typed generic strategies", () => {
  for (
    const strategy of [
      'case "progress_ring"',
      'case "glyph_cycle"',
      'case "terminal_prompt"',
      'case "asset_signal"',
      'case "asset_pulse"',
    ]
  ) {
    assertEquals(surfaceSource.includes(strategy), true);
  }
  for (
    const provider of [
      "claude-code",
      "codex-deepseek",
      "gemini",
      "grok",
    ]
  ) {
    assertEquals(surfaceSource.includes(provider), false);
  }
});

Deno.test("Provider UI v1 loading uses a compact neutral compatibility fallback", () => {
  assertEquals(
    surfaceSource.includes(
      'slot === "loading" && manifest.ui.schema_version === 1',
    ),
    true,
  );
  assertEquals(surfaceSource.includes("<LegacyProviderActivity />"), true);
  assertEquals(surfaceSource.includes("const legacyResponsive"), true);
});
