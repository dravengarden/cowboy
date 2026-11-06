import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import type { GitChangeTarget } from "./codeApi.ts";
import {
  comparisonFetchable,
  resolveReviewComparison,
  reviewCompareBranches,
  reviewCompareLabel,
  reviewCompareScenarios,
  reviewFetchAge,
} from "./reviewCompareModel.ts";

const main = { reference: "refs/remotes/origin/main", label: "origin/main" };
const topic = { reference: "refs/remotes/origin/topic", label: "origin/topic" };
const other = { reference: "refs/heads/other", label: "other" };
const comparisons = [main, topic, other];
const target: GitChangeTarget = {
  changeCount: 1,
  upstream: { ...topic, ahead: 2, behind: 0 },
  base: { ...main, ahead: 4, behind: 9 },
};

test("scenarios name the reason before the branch", () => {
  const scenarios = reviewCompareScenarios(target, comparisons, main.reference);
  assertEquals(scenarios.map((item) => item.key), [
    "uncommitted",
    "merge",
    "unpushed",
  ]);
  assertEquals(
    reviewCompareLabel(main.reference, scenarios, comparisons),
    "Ready to merge · origin/main",
  );
  assertEquals(
    reviewCompareLabel(null, scenarios, comparisons),
    "Uncommitted",
  );
  assertEquals(
    reviewCompareLabel(other.reference, scenarios, comparisons),
    "Branch · other",
  );
});

test("an upstream equal to the base is not offered twice", () => {
  const scenarios = reviewCompareScenarios(
    { changeCount: 0, upstream: { ...main, ahead: 0, behind: 0 } },
    comparisons,
    main.reference,
  );
  assertEquals(scenarios.map((item) => item.key), ["uncommitted", "merge"]);
});

test("a repository without a default still offers uncommitted work", () => {
  assertEquals(
    reviewCompareScenarios(undefined, [], undefined).map((item) => item.key),
    ["uncommitted"],
  );
});

test("the branch list skips scenarios and honours the query", () => {
  const scenarios = reviewCompareScenarios(target, comparisons, main.reference);
  assertEquals(reviewCompareBranches(comparisons, scenarios, ""), [other]);
  assertEquals(reviewCompareBranches(comparisons, scenarios, "ZZ"), []);
});

test("a stored comparison survives only while it exists", () => {
  assertEquals(
    resolveReviewComparison(other.reference, comparisons, main.reference),
    other.reference,
  );
  assertEquals(
    resolveReviewComparison("refs/heads/gone", comparisons, main.reference),
    main.reference,
  );
  assertEquals(resolveReviewComparison(null, comparisons, main.reference), null);
  assertEquals(
    resolveReviewComparison(undefined, comparisons, main.reference),
    main.reference,
  );
  assertEquals(resolveReviewComparison(undefined, [], undefined), null);
});

test("Not pushed stops at HEAD where the Machine can", () => {
  const scenarios = reviewCompareScenarios(
    target,
    comparisons,
    main.reference,
    true,
  );
  const unpushed = scenarios.find((item) => item.key === "unpushed")!;
  assertEquals(unpushed.comparison, `committed:${topic.reference}`);
  assertEquals(unpushed.detail, "Commits not on origin/topic yet");
  assertEquals(
    reviewCompareLabel(unpushed.comparison, scenarios, comparisons),
    "Not pushed · origin/topic",
  );
  assertEquals(
    resolveReviewComparison(unpushed.comparison, comparisons, main.reference),
    unpushed.comparison,
  );
});

test("fetch age stays short and only remote branches fetch", () => {
  assertEquals(reviewFetchAge(1_000, 1_000), "Fetched just now");
  assertEquals(reviewFetchAge(0, 3 * 60_000), "Fetched 3m ago");
  assertEquals(reviewFetchAge(0, 2 * 3_600_000), "Fetched 2h ago");
  assertEquals(reviewFetchAge(0, 50 * 3_600_000), "Fetched 2d ago");
  assertEquals(comparisonFetchable(main.reference), true);
  assertEquals(comparisonFetchable(`committed:${main.reference}`), true);
  assertEquals(comparisonFetchable(other.reference), false);
  assertEquals(comparisonFetchable(null), false);
});

test("a recorded session start is offered as a scenario, not a branch", () => {
  const start = {
    reference: "refs/cowboy/session-bases/sess-1",
    label: "session start",
  };
  const all = [...comparisons, start];
  const scenarios = reviewCompareScenarios(target, all, main.reference);
  assertEquals(scenarios.map((item) => item.key), [
    "uncommitted",
    "session",
    "merge",
    "unpushed",
  ]);
  assertEquals(reviewCompareLabel(start.reference, scenarios, all), "This session");
  assertEquals(reviewCompareBranches(all, scenarios, ""), [other]);
});
