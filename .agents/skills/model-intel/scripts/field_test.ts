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
  assertEquals(groups.map((group) => [group.target, group.n]), [
    ["gpt-6.2-sol:max", 3],
    ["gpt-6.2-astra:max", 1],
  ]);
  const sol = groups[0]!;
  assertEquals(sol.with_usage, 2);
  assertEquals(sol.median_tokens, 200);
  assertEquals(sol.mean_findings, 3);
  assertEquals(sol.verdicts, { approve: 1, "needs-attention": 1, none: 1 });
  assertEquals(sol.risk, { routine: 2, undeclared: 1 });
});
