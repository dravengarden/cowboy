import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { compactTokens, contextModel, modelUsageSummary, sessionModelUsage } from "./modelUsage";

test("model usage reads the worker's totals and ignores malformed rows", () => {
  const session = {
    usage: {
      used: 1,
      size: 2,
      observed_at_ms: 3,
      raw: {
        _meta: { "_claude/model": " claude-opus-4-5 " },
        _cowboy: {
          modelUsage: [
            { model: "small", totalTokens: 10, inputTokens: 8, outputTokens: 2, turns: 1 },
            { model: "big", totalTokens: 5000, turns: 4 },
            { model: "", totalTokens: 9 },
            { model: "zero", totalTokens: 0 },
            "junk",
          ],
        },
      },
    },
  };
  const rows = sessionModelUsage(session);
  assertEquals(rows.map((row) => row.model), ["big", "small"]);
  assertEquals(rows[1], { model: "small", totalTokens: 10, inputTokens: 8, outputTokens: 2, turns: 1 });
  assertEquals(contextModel(session), "claude-opus-4-5");
  assertEquals(sessionModelUsage({}), []);
  assertEquals(contextModel({}), null);
  assertEquals(modelUsageSummary(rows[0]!), "5k tokens · 4 turns");
  assertEquals(modelUsageSummary({ ...rows[1]!, turns: 1 }), "10 tokens · 1 turn");
});

test("token counts stay short", () => {
  assertEquals(compactTokens(950), "950");
  assertEquals(compactTokens(1000), "1k");
  assertEquals(compactTokens(12_345), "12k");
  assertEquals(compactTokens(1_250), "1.3k");
  assertEquals(compactTokens(999_999), "1M");
  assertEquals(compactTokens(4_100_000), "4.1M");
});
