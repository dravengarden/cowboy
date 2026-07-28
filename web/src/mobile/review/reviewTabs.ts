import type { CodeDiffScope } from "./codeApi";

const STORAGE_KEY = "cowboy:code-review-tabs:v1";
const MAX_TABS_PER_MODE = 12;

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

export function reorderReviewTabs(
  tabs: readonly ReviewTab[],
  movingKey: string,
  targetKey: string,
): ReviewTab[] {
  if (movingKey === targetKey) return [...tabs];
  const from = tabs.findIndex((tab) => reviewTabKey(tab) === movingKey);
  const to = tabs.findIndex((tab) => reviewTabKey(tab) === targetKey);
  if (from < 0 || to < 0) return [...tabs];
  const next = [...tabs];
  const [moving] = next.splice(from, 1);
  if (!moving) return [...tabs];
  next.splice(to, 0, moving);
  return next;
}

export function openReviewTab(
  tabs: readonly ReviewTab[],
  next: ReviewTab,
): ReviewTab[] {
  const key = reviewTabKey(next);
  const existing = tabs.find((tab) => reviewTabKey(tab) === key);
  if (existing) {
    return [...tabs];
  }
  const opened = [...tabs, next];
  const sameMode = (tab: ReviewTab): boolean => tab.kind === next.kind;
  if (opened.filter(sameMode).length <= MAX_TABS_PER_MODE) return opened;
  const evict = opened.findIndex((tab) => sameMode(tab) && !tab.pinned);
  if (evict < 0) {
    const firstInMode = opened.findIndex(sameMode);
    return opened.filter((_, index) => index !== firstInMode);
  }
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

export function retainChangedDiffTabs(
  tabs: readonly ReviewTab[],
  changedKeys: ReadonlySet<string>,
): ReviewTab[] {
  return tabs.filter((tab) =>
    tab.kind === "source" || changedKeys.has(reviewTabKey(tab))
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
    const tabs = value.flatMap((candidate): ReviewTab[] => {
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
    });
    return [
      ...tabs.filter((tab) => tab.kind === "source").slice(
        -MAX_TABS_PER_MODE,
      ),
      ...tabs.filter((tab) => tab.kind === "diff").slice(-MAX_TABS_PER_MODE),
    ];
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
    else {
      all[sessionId] = [
        ...tabs.filter((tab) => tab.kind === "source").slice(
          -MAX_TABS_PER_MODE,
        ),
        ...tabs.filter((tab) => tab.kind === "diff").slice(
          -MAX_TABS_PER_MODE,
        ),
      ];
    }
    target.setItem(STORAGE_KEY, JSON.stringify(all));
  } catch {
    // Tab restoration is a convenience; storage denial must not break review.
  }
}
