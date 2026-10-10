import { readdirSync, readFileSync } from "node:fs";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import {
  applyOccupancyHostPlugins,
  occupancyProviderIds,
} from "./occupancyHostMap.ts";

function firstPartyHosts(): Array<{ id: string } & Record<string, unknown>> {
  const root = new URL("../../plugins/", import.meta.url);
  return [...readdirSync(root, { withFileTypes: true })]
    .filter((entry) => entry.isDirectory())
    .sort((left, right) => left.name.localeCompare(right.name))
    .flatMap((entry) => {
      try {
        const host = JSON.parse(
          readFileSync(new URL(`${entry.name}/host.json`, root), "utf8"),
        ) as Record<string, unknown>;
        return [{ id: entry.name, ...host }];
      } catch (reason) {
        if ((reason as { code?: string }).code === "ENOENT") return [];
        throw reason;
      }
    });
}

const FIRST_PARTY_HOSTS = firstPartyHosts();
applyOccupancyHostPlugins(FIRST_PARTY_HOSTS);

test("adapter occupancy has no source-compiled first-party inventory", () => {
  const source = readFileSync(
    new URL("./occupancyHostMap.ts", import.meta.url), "utf8",
  );
  assertEquals(source.includes("bundledHostPlugins"), false);
  assertEquals(source.includes("FALLBACK_ADAPTER_ALIASES"), false);
  assertEquals(source.includes('"claude-code"'), false);
  assert(FIRST_PARTY_HOSTS.length > 0);
});

test("adapter slots include the slot id and first-party aliases", () => {
  assertEquals(occupancyProviderIds("claude"), [
    "claude",
    "claude-code",
    "claude-deepseek",
  ]);
  assertEquals(occupancyProviderIds("codex"), ["codex", "codex-deepseek"]);
});

test("activated host inventory replaces adapter-slot occupancy", () => {
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
