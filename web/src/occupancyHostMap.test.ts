import { assert, assertEquals } from "jsr:@std/assert";
import {
  applyOccupancyHostPlugins,
  occupancyProviderIds,
} from "./occupancyHostMap.ts";

Deno.test("adapter occupancy is generated from bundled host.json", () => {
  const source = Deno.readTextFileSync(
    new URL("./occupancyHostMap.ts", import.meta.url),
  );
  assert(source.includes("bundledHostPlugins"));
  assertEquals(source.includes("FALLBACK_ADAPTER_ALIASES"), false);
  assertEquals(source.includes('"claude-code"'), false);
});

Deno.test("adapter slots include the slot id and first-party aliases", () => {
  assertEquals(occupancyProviderIds("claude"), [
    "claude",
    "claude-code",
    "claude-deepseek",
  ]);
  assertEquals(occupancyProviderIds("codex"), ["codex", "codex-deepseek"]);
});

Deno.test("activated host plugins overlay adapter-slot occupancy", () => {
  try {
    applyOccupancyHostPlugins([
      { id: "custom-claude", adapter_slot: "claude" },
      { id: "password", slots: ["login.method"] },
    ]);
    assertEquals(occupancyProviderIds("claude"), ["claude", "custom-claude"]);
    assertEquals(occupancyProviderIds("codex"), ["codex", "codex-deepseek"]);
  } finally {
    applyOccupancyHostPlugins([]);
  }
  assertEquals(occupancyProviderIds("claude"), [
    "claude",
    "claude-code",
    "claude-deepseek",
  ]);
  assertEquals(occupancyProviderIds("codex"), ["codex", "codex-deepseek"]);
});
