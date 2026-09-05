import { assert, assertEquals } from "jsr:@std/assert";
import {
  applyOccupancyHostPlugins,
  occupancyProviderIds,
} from "./occupancyHostMap.ts";

function firstPartyHosts(): Array<{ id: string } & Record<string, unknown>> {
  const root = new URL("../../plugins/", import.meta.url);
  return [...Deno.readDirSync(root)]
    .filter((entry) => entry.isDirectory)
    .sort((left, right) => left.name.localeCompare(right.name))
    .flatMap((entry) => {
      try {
        const host = JSON.parse(
          Deno.readTextFileSync(new URL(`${entry.name}/host.json`, root)),
        ) as Record<string, unknown>;
        return [{ id: entry.name, ...host }];
      } catch (reason) {
        if (reason instanceof Deno.errors.NotFound) return [];
        throw reason;
      }
    });
}

const FIRST_PARTY_HOSTS = firstPartyHosts();
applyOccupancyHostPlugins(FIRST_PARTY_HOSTS);

Deno.test("adapter occupancy has no source-compiled first-party inventory", () => {
  const source = Deno.readTextFileSync(
    new URL("./occupancyHostMap.ts", import.meta.url),
  );
  assertEquals(source.includes("bundledHostPlugins"), false);
  assertEquals(source.includes("FALLBACK_ADAPTER_ALIASES"), false);
  assertEquals(source.includes('"claude-code"'), false);
  assert(FIRST_PARTY_HOSTS.length > 0);
});

Deno.test("adapter slots include the slot id and first-party aliases", () => {
  assertEquals(occupancyProviderIds("claude"), [
    "claude",
    "claude-code",
    "claude-deepseek",
  ]);
  assertEquals(occupancyProviderIds("codex"), ["codex", "codex-deepseek"]);
});

Deno.test("activated host inventory replaces adapter-slot occupancy", () => {
  try {
    applyOccupancyHostPlugins([
      { id: "custom-claude", adapter_slot: "claude" },
      { id: "password", slots: ["login.method"] },
    ]);
    assertEquals(occupancyProviderIds("claude"), ["claude", "custom-claude"]);
    assertEquals(occupancyProviderIds("codex"), ["codex"]);
  } finally {
    applyOccupancyHostPlugins(FIRST_PARTY_HOSTS);
  }
  assertEquals(occupancyProviderIds("claude"), [
    "claude",
    "claude-code",
    "claude-deepseek",
  ]);
  assertEquals(occupancyProviderIds("codex"), ["codex", "codex-deepseek"]);
});
