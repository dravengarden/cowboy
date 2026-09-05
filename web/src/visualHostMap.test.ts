import { assert, assertEquals } from "jsr:@std/assert";
import {
  applyVisualHostPlugins,
  providerSurfaceColor,
} from "./visualHostMap.ts";

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
applyVisualHostPlugins(FIRST_PARTY_HOSTS);

Deno.test("Provider colors have no source-compiled first-party inventory", () => {
  const source = Deno.readTextFileSync(
    new URL("./visualHostMap.ts", import.meta.url),
  );
  assertEquals(source.includes("bundledHostPlugins"), false);
  assertEquals(source.includes("#E08A6A"), false);
  assertEquals(source.includes("#E8E4DC"), false);
  assert(FIRST_PARTY_HOSTS.length > 0);
  assertEquals(providerSurfaceColor("grok")?.dark.primary, "#E8E4DC");
  assertEquals(providerSurfaceColor("claude-code")?.dark.primary, "#E08A6A");
});

Deno.test("activated host inventory replaces Provider surface colors", () => {
  try {
    applyVisualHostPlugins([{
      id: "grok",
      visual: {
        light: { primary: "#111111", secondary: "#222222" },
        dark: { primary: "#EEEEEE", secondary: "#DDDDDD" },
      },
    }]);
    assertEquals(providerSurfaceColor("grok")?.dark.primary, "#EEEEEE");
    assertEquals(providerSurfaceColor("claude-code"), undefined);
  } finally {
    applyVisualHostPlugins(FIRST_PARTY_HOSTS);
  }
  assertEquals(providerSurfaceColor("grok")?.dark.primary, "#E8E4DC");
});

Deno.test("Provider colors resolve the exact release before the default", () => {
  const oldDigest = `sha256:${"a".repeat(64)}`;
  const currentDigest = `sha256:${"b".repeat(64)}`;
  try {
    applyVisualHostPlugins([
      {
        id: "future",
        plugin_version: "1.0.0",
        artifact_digest: oldDigest,
        default_for_id: false,
        visual: {
          light: { primary: "#111111", secondary: "#222222" },
          dark: { primary: "#333333", secondary: "#444444" },
        },
      },
      {
        id: "future",
        plugin_version: "2.0.0",
        artifact_digest: currentDigest,
        default_for_id: true,
        visual: {
          light: { primary: "#AAAAAA", secondary: "#BBBBBB" },
          dark: { primary: "#CCCCCC", secondary: "#DDDDDD" },
        },
      },
    ]);
    assertEquals(
      providerSurfaceColor("future", "1.0.0", oldDigest)?.dark.primary,
      "#333333",
    );
    assertEquals(providerSurfaceColor("future")?.dark.primary, "#CCCCCC");
    assertEquals(
      providerSurfaceColor(
        "future",
        "1.0.0",
        `sha256:${"c".repeat(64)}`,
      ),
      undefined,
    );
  } finally {
    applyVisualHostPlugins(FIRST_PARTY_HOSTS);
  }
});
