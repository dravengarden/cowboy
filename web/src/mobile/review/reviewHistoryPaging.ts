import type { GitCommitSummary } from "./codeApi";

export const HISTORY_PAGE_SIZE = 128;

export function mergeHistoryPage(
  current: readonly GitCommitSummary[],
  page: readonly GitCommitSummary[],
  pageTruncated: boolean,
): { commits: GitCommitSummary[]; truncated: boolean } {
  const seen = new Set(current.map((commit) => commit.oid));
  const appended = page.filter((commit) => !seen.has(commit.oid));
  return {
    commits: appended.length === 0 ? [...current] : [...current, ...appended],
    // An adapter that ignores `after` repeats the first page. Stop so the
    // sentinel cannot loop.
    truncated: pageTruncated && appended.length > 0,
  };
}

export function historyPageCursor(
  commits: readonly GitCommitSummary[],
): string | undefined {
  return commits.at(-1)?.oid;
}
