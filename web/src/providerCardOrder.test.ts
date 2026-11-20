import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  namedOnlyPlacement,
  providerCardRank,
  sortByProviderCardOrder,
} from "./providerCardOrder.ts";

const row = (key: string, ...providers: string[]) => ({ key, providers });

test("Provider cards follow the deliberate order, not the catalog's", () => {
  const catalog = ["antigravity", "claude-code", "codex", "github", "grok", "zed"];
  assertEquals(sortByProviderCardOrder(catalog, (id) => [id]), [
    "claude-code",
    "antigravity",
    "codex",
    "grok",
    "github",
    "zed",
  ]);
  // A shared credential ranks by its highest-ranked Provider.
  assertEquals(
    sortByProviderCardOrder([row("c", "codex", "codex-deepseek"), row("a", "claude-code")], (r) =>
      r.providers
    ).map((r) => r.key),
    ["a", "c"],
  );
  assertEquals(providerCardRank("unknown") > providerCardRank("grok"), true);
});

test("Antigravity's named-only account cards sit after Claude and before Codex", () => {
  const rows = [
    row("claude-code", "claude-code"),
    row("claude-deepseek", "claude-deepseek"),
    row("codex", "codex"),
    row("grok", "grok"),
  ];
  const placement = namedOnlyPlacement(rows, ["antigravity", "future"]);
  assertEquals([...placement.before], [["codex", ["antigravity"]]]);
  assertEquals(placement.trailing, ["future"]);
  // Without a later row the cards keep the trailing place.
  assertEquals(namedOnlyPlacement([row("claude-code", "claude-code")], ["antigravity"]), {
    before: new Map(),
    trailing: ["antigravity"],
  });
});
