import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { costLabel, rowLabel } from "./ReviewRoutingTable";
import type { ReviewRouting } from "./agentTools";

const table: ReviewRouting = {
  evidence: "2026-11-21",
  providers: {
    codex: {
      high: { auto: [{ family: "astra", model: "gpt-6.2-astra", efforts: ["max"] }] },
      routine: { auto: [{ family: "sol", model: "gpt-6.2-sol", efforts: ["max"] }] },
    },
    "claude-code": {
      high: { auto: [{ family: "opus", efforts: ["high"] }] },
      routine: { auto: [{ family: "opus", efforts: ["high"] }] },
    },
  },
  costs: {
    "gpt-6.2-sol:max": {
      median_tokens: 3220000,
      n: 4,
      serious_recall: "12/14",
      field: { purpose: "review", median_tokens: 2406303, median_minutes: 7, n: 6 },
    },
    "opus:high": { median_tokens: null },
  },
};

test("routing rows read as model and effort with their measured cost", () => {
  const routine = table.providers.codex!.routine!.auto[0]!;
  assertEquals(rowLabel(routine), "gpt-6.2-sol · max");
  assertEquals(costLabel(routine, table.costs), "2.4M tokens, 7 min (n=6) · pilot 12/14");
  const opus = table.providers["claude-code"]!.routine!.auto[0]!;
  assertEquals(rowLabel(opus), "Opus · high");
  assertEquals(costLabel(opus, table.costs), "cost unknown");
  assertEquals(
    costLabel({ family: "sol", model: "gpt-9-sol", efforts: ["max"] }, {}),
    "cost unknown",
  );
});

test("analysis-only providers format model, thought_level effort and field cost", () => {
  const row = { family: "gemini-hydrogen", model: "gemini-4-hydrogen", efforts: ["high"] };
  const costs = {
    "gemini-4-hydrogen:high": {
      median_tokens: null,
      field: { purpose: "analysis", median_tokens: 59522, median_minutes: 1, n: 43 },
    },
  };
  assertEquals(rowLabel(row), "gemini-4-hydrogen · high");
  assertEquals(costLabel(row, costs), "60k tokens, 1 min (n=43)");
});
