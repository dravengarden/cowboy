import type { GitComparison, GitChangeTarget } from "./codeApi.ts";

/** A reason to look at changes, offered before the branch it resolves to. */
export interface ReviewCompareScenario {
  readonly key: "uncommitted" | "merge" | "unpushed";
  readonly title: string;
  readonly detail: string;
  readonly comparison: string | null;
}

const COMMITTED = "committed:";

/** The Git reference a comparison starts from. */
export function comparisonReference(comparison: string): string {
  return comparison.startsWith(COMMITTED)
    ? comparison.slice(COMMITTED.length)
    : comparison;
}

/**
 * Branch comparisons run from the merge base to the working tree, so a branch
 * scenario includes uncommitted work and says so. Not pushed stops at HEAD
 * where the Machine can; an older one keeps the wider comparison and says so.
 */
export function reviewCompareScenarios(
  target: GitChangeTarget | undefined,
  comparisons: readonly GitComparison[],
  defaultComparison: string | undefined,
  committedComparisons = false,
): ReviewCompareScenario[] {
  const scenarios: ReviewCompareScenario[] = [{
    key: "uncommitted",
    title: "Uncommitted",
    detail: "Staged and unstaged work",
    comparison: null,
  }];
  const label = (reference: string): string | undefined =>
    comparisons.find((item) => item.reference === reference)?.label;
  const base = defaultComparison ? label(defaultComparison) : undefined;
  if (defaultComparison && base) {
    scenarios.push({
      key: "merge",
      title: "Ready to merge",
      detail: `Everything since ${base}, including uncommitted work`,
      comparison: defaultComparison,
    });
  }
  const upstream = target?.upstream?.reference;
  const upstreamLabel = upstream ? label(upstream) : undefined;
  if (upstream && upstreamLabel && upstream !== defaultComparison) {
    scenarios.push({
      key: "unpushed",
      title: "Not pushed",
      detail: committedComparisons
        ? `Commits not on ${upstreamLabel} yet`
        : `Everything since ${upstreamLabel}, including uncommitted work`,
      comparison: committedComparisons ? `${COMMITTED}${upstream}` : upstream,
    });
  }
  return scenarios;
}

/** What the current comparison means, in the words of its scenario. */
export function reviewCompareLabel(
  comparison: string | null | undefined,
  scenarios: readonly ReviewCompareScenario[],
  comparisons: readonly GitComparison[],
): string {
  if (comparison === undefined) return "Loading comparison";
  const reference = comparison === null ? null : comparisonReference(comparison);
  const branch = reference === null
    ? undefined
    : comparisons.find((item) => item.reference === reference)?.label ??
      reference;
  const scenario = scenarios.find((item) => item.comparison === comparison);
  if (!scenario) return `Branch · ${branch}`;
  return branch ? `${scenario.title} · ${branch}` : scenario.title;
}

/** Branches not already offered as a scenario, narrowed by a typed query. */
export function reviewCompareBranches(
  comparisons: readonly GitComparison[],
  scenarios: readonly ReviewCompareScenario[],
  query: string,
): GitComparison[] {
  const offered = new Set(scenarios.map((item) => item.comparison));
  const needle = query.trim().toLowerCase();
  return comparisons.filter((item) =>
    !offered.has(item.reference) &&
    (!needle || item.label.toLowerCase().includes(needle))
  );
}

export function comparisonAvailable(
  comparison: string,
  comparisons: readonly GitComparison[],
): boolean {
  const reference = comparisonReference(comparison);
  return comparisons.some((item) => item.reference === reference);
}

/**
 * The stored choice decides only when it can still be honoured: a deleted
 * branch falls back to the repository's default instead of failing the list.
 */
export function resolveReviewComparison(
  preferred: string | null | undefined,
  comparisons: readonly GitComparison[],
  defaultComparison: string | undefined,
): string | null {
  if (preferred === null) return null;
  if (preferred !== undefined && comparisonAvailable(preferred, comparisons)) {
    return preferred;
  }
  return defaultComparison ?? null;
}

/** How old the compared branch is, in the fewest words that stay honest. */
export function reviewFetchAge(fetchedAtMs: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - fetchedAtMs) / 60_000);
  if (minutes < 1) return "Fetched just now";
  if (minutes < 60) return `Fetched ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  return hours < 24
    ? `Fetched ${hours}h ago`
    : `Fetched ${Math.floor(hours / 24)}d ago`;
}

/** Only a remote branch can be fetched. */
export function comparisonFetchable(comparison: string | null | undefined): boolean {
  return !!comparison && comparisonReference(comparison).startsWith("refs/remotes/");
}
