const STORAGE_KEY = "cowboy:code-review-mode:v1";

export type ReviewMode = "files" | "git";

function storage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

function allModes(): Record<string, unknown> {
  try {
    const parsed = JSON.parse(storage()?.getItem(STORAGE_KEY) ?? "{}");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : {};
  } catch {
    return {};
  }
}

export function normalizeReviewMode(value: unknown): ReviewMode {
  if (value === "files" || value === "code") return "files";
  return "git";
}

export function loadReviewMode(sessionId: string): ReviewMode {
  return normalizeReviewMode(allModes()[sessionId]);
}

export function saveReviewMode(sessionId: string, mode: ReviewMode): void {
  const target = storage();
  if (!target) return;
  try {
    target.setItem(
      STORAGE_KEY,
      JSON.stringify({ ...allModes(), [sessionId]: mode }),
    );
  } catch {
    // A local preference must never make Code Review unavailable.
  }
}
