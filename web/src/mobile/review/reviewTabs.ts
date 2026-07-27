import type { CodeDiffScope } from "./codeApi";

const STORAGE_KEY = "cowboy:code-review-tabs:v1";
const MAX_TABS = 12;

export type ReviewTab =
  | { kind: "source"; path: string; pinned: boolean }
  | {
    kind: "diff";
    path: string;
    scope: CodeDiffScope;
    pinned: boolean;
  };

export function reviewTabKey(tab: ReviewTab): string {
  return tab.kind === "source"
    ? `source:${tab.path}`
    : `diff:${tab.scope}:${tab.path}`;
}

export function openReviewTab(
  tabs: readonly ReviewTab[],
  next: ReviewTab,
): ReviewTab[] {
  const key = reviewTabKey(next);
  const existing = tabs.find((tab) => reviewTabKey(tab) === key);
  if (existing) {
    return [...tabs.filter((tab) => reviewTabKey(tab) !== key), existing];
  }
  const opened = [...tabs, next];
  if (opened.length <= MAX_TABS) return opened;
  const evict = opened.findIndex((tab) => !tab.pinned);
  if (evict < 0) return opened.slice(-MAX_TABS);
  return opened.filter((_, index) => index !== evict);
}

export function closeReviewTab(
  tabs: readonly ReviewTab[],
  key: string,
): ReviewTab[] {
  return tabs.filter((tab) => reviewTabKey(tab) !== key);
}

export function closeOtherReviewTabs(
  tabs: readonly ReviewTab[],
  key: string,
): ReviewTab[] {
  return tabs.filter((tab) => tab.pinned || reviewTabKey(tab) === key);
}

export function toggleReviewTabPin(
  tabs: readonly ReviewTab[],
  key: string,
): ReviewTab[] {
  return tabs.map((tab) =>
    reviewTabKey(tab) === key ? { ...tab, pinned: !tab.pinned } : tab
  );
}

function storage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

export function loadReviewTabs(sessionId: string): ReviewTab[] {
  try {
    const all = JSON.parse(storage()?.getItem(STORAGE_KEY) ?? "{}") as Record<
      string,
      unknown
    >;
    const value = all[sessionId];
    if (!Array.isArray(value)) return [];
    return value.flatMap((candidate): ReviewTab[] => {
      if (
        !candidate || typeof candidate !== "object" ||
        typeof candidate.path !== "string" ||
        typeof candidate.pinned !== "boolean"
      ) return [];
      if (candidate.kind === "source") {
        return [{
          kind: "source",
          path: candidate.path,
          pinned: candidate.pinned,
        }];
      }
      if (
        candidate.kind === "diff" &&
        ["staged", "unstaged", "combined"].includes(candidate.scope)
      ) {
        return [{
          kind: "diff",
          path: candidate.path,
          scope: candidate.scope as CodeDiffScope,
          pinned: candidate.pinned,
        }];
      }
      return [];
    }).slice(-MAX_TABS);
  } catch {
    return [];
  }
}

export function saveReviewTabs(sessionId: string, tabs: ReviewTab[]): void {
  const target = storage();
  if (!target) return;
  try {
    const all = JSON.parse(target.getItem(STORAGE_KEY) ?? "{}") as Record<
      string,
      ReviewTab[]
    >;
    if (tabs.length === 0) delete all[sessionId];
    else all[sessionId] = tabs.slice(-MAX_TABS);
    target.setItem(STORAGE_KEY, JSON.stringify(all));
  } catch {
    // Tab restoration is a convenience; storage denial must not break review.
  }
}
