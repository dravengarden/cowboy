import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { GitChangeTarget } from "./codeApi.ts";
import {
  resolveReviewComparison,
  reviewCompareBranches,
  reviewCompareLabel,
  reviewCompareScenarios,
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
