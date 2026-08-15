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

Deno.test("Provider management cards keep geometry in the Cowboy component library", () => {
  assertEquals(
    managementSource.includes("data-provider-management-identity"),
    true,
  );
  assertEquals(
    managementSource.includes(
      'gridTemplateColumns: "32px minmax(0, 1fr) auto"',
    ),
    true,
  );
  assertEquals(
    managementSource.includes(
      'slot={scope === "service" ? "information" : "card"}',
    ),
    false,
  );
  for (const slot of ['slot="setup"', 'slot="empty"', 'slot="settings"']) {
    assertEquals(managementSource.includes(slot), true);
  }
  assertEquals(
    managementSource.includes(
      'type ProviderManagementLifecycleSlot = "setup" | "empty" | "settings"',
    ),
    true,
  );
  assertEquals(
    managementSource.includes("data-provider-management-actions"),
    true,
  );
  assertEquals(managementSource.includes("WebkitLineClamp: 2"), true);
});

Deno.test("Provider marks preserve host component classes and compact chip spacing", () => {
  assertEquals(surfaceSource.includes("className?: string | undefined;"), true);
  assertEquals(surfaceSource.includes("className={className}"), true);
  assertEquals(
    managementSource.includes('"& .MuiChip-icon": { ml: 0.625, mr: 0.125 }'),
    true,
  );
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
