const STORAGE_KEY = "cowboy:code-review-progress:v1";

export type ReviewProgress = Record<string, string>;

function storage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

function allProgress(): Record<string, ReviewProgress> {
  try {
    const parsed = JSON.parse(storage()?.getItem(STORAGE_KEY) ?? "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }
    return parsed as Record<string, ReviewProgress>;
  } catch {
    return {};
  }
}

export function loadReviewProgress(sessionId: string): ReviewProgress {
  const progress = allProgress()[sessionId];
  if (!progress || typeof progress !== "object" || Array.isArray(progress)) {
    return {};
  }
  return Object.fromEntries(
    Object.entries(progress).filter((entry) =>
      typeof entry[0] === "string" && typeof entry[1] === "string"
    ),
  );
}

export function saveReviewProgress(
  sessionId: string,
  progress: ReviewProgress,
): void {
  const target = storage();
  if (!target) return;
  try {
    const all = allProgress();
    if (Object.keys(progress).length === 0) delete all[sessionId];
    else all[sessionId] = progress;
    target.setItem(STORAGE_KEY, JSON.stringify(all));
  } catch {
    // Review progress is a convenience; storage denial must not break review.
  }
}

export function revisionMatches(
  progress: ReviewProgress,
  key: string,
  revision: string | undefined,
): boolean {
  return revision !== undefined && progress[key] === revision;
}
