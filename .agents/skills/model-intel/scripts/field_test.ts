import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { aggregate, type Call } from "./field.ts";

const call = (overrides: Partial<Call>): Call => ({
  provider: "codex",
  purpose: "review",
  model: "gpt-6.2-sol",
  effort: "max",
  risk: "routine",
  tokens: 100,
  uncached: 10,
  minutes: 1,
  verdict: "approve",
  findings: 0,
  ...overrides,
});

test("field groups by provider, purpose and resolved target and ignores missing usage", () => {
  const groups = aggregate([
    call({ tokens: 100, findings: 2 }),
    call({ tokens: 300, findings: 4, verdict: "needs-attention" }),
    call({ tokens: null, findings: null, verdict: null, risk: "undeclared" }),
    call({ model: "gpt-6.2-astra", tokens: 900 }),
  ]);
  assertEquals(
    groups.map((group) => [group.target, group.n]),
    [
      ["gpt-6.2-sol:max", 3],
      ["gpt-6.2-astra:max", 1],
    ],
  );
  const sol = groups[0]!;
  assertEquals(sol.with_usage, 2);
  assertEquals(sol.median_tokens, 200);
  assertEquals(sol.mean_findings, 3);
  assertEquals(sol.verdicts, { approve: 1, "needs-attention": 1, none: 1 });
  assertEquals(sol.risk, { routine: 2, undeclared: 1 });
});

import { detectCostDrifts, summarizeTelemetry } from "./field.ts";
import type { RoutingTable } from "./score.ts";

test("summarizeTelemetry splits review and analysis calls and detects cost drift", () => {
  const routing: RoutingTable = {
    schema: 1,
    evidence: "2026-11-21",
    providers: {},
    costs: {
      "gpt-6.2-sol:max": {
        field: { median_tokens: 1000, n: 10 },
      },
    },
  };
  const calls = [
    ...Array.from({ length: 5 }, () => call({ tokens: 1500, selection: "auto", risk: "routine" })),
    call({
      provider: "antigravity",
      purpose: "analysis",
      model: "gemini-4-hydrogen",
      effort: "high",
      tokens: 90,
      selection: "explicit",
      risk: "undeclared",
      verdict: null,
      findings: null,
    }),
  ];
  const summary = summarizeTelemetry(calls, routing, { minSamples: 5, threshold: 0.3 });
  assertEquals(summary.review_groups.length, 1);
  assertEquals(summary.analysis_groups.length, 1);
  assertEquals(summary.analysis_groups[0]?.target, "gemini-4-hydrogen:high");
  assertEquals(summary.review_selection, { auto: 5 });
  assertEquals(summary.cost_drifts, [
    {
      target: "gpt-6.2-sol:max",
      recorded_median_tokens: 1000,
      live_median_tokens: 1500,
      delta_ratio: 0.5,
      n: 5,
    },
  ]);
  // Below minSamples produces no drift alert.
  assertEquals(
    detectCostDrifts(summary.review_groups, routing, { minSamples: 10, threshold: 0.3 }),
    [],
  );
});
