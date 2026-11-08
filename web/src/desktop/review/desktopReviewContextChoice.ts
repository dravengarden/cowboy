// Which code Desktop Review reads for a Session, when the reader picked one
// (docs/desktop-code-review.md D1b): a review-directory value
// (`session::<id>`, `project`, `workspace::<machine>::<id>`,
// `worktree::…`), resolved like the folder default. Per device and never
// synced; reading another context never moves the Agent.

import { useSyncExternalStore } from "react";

const STORAGE_KEY = "cowboy.desktop.reviewContext.v1";
export const REVIEW_CONTEXT_CHOICE_LIMIT = 100;

type Choices = Readonly<Record<string, string>>;

function read(): Choices {
  try {
    const parsed: unknown = JSON.parse(globalThis.localStorage?.getItem(STORAGE_KEY) ?? "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    return Object.fromEntries(
      Object.entries(parsed).filter((entry): entry is [string, string] =>
        typeof entry[1] === "string" && entry[1].length > 0 && entry[1].length < 4096
      ),
    );
  } catch {
    return {};
  }
}

let cache: Choices | null = null;
const listeners = new Set<() => void>();

function choices(): Choices {
  cache ??= read();
  return cache;
}

/** The stored choices with `session` set to `value` (or cleared), newest
 *  last and bounded. */
export function withReviewContextChoice(current: Choices, session: string, value: string | null): Choices {
  const entries = Object.entries(current).filter(([id]) => id !== session);
  if (value) entries.push([session, value]);
  return Object.fromEntries(entries.slice(-REVIEW_CONTEXT_CHOICE_LIMIT));
}

export function setReviewContextChoice(session: string, value: string | null): void {
  cache = withReviewContextChoice(choices(), session, value);
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(cache));
  } catch {
    // The choice then lasts for this page only.
  }
  for (const listener of listeners) listener();
}

export function useReviewContextChoice(session: string | undefined): string | undefined {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => session ? choices()[session] : undefined,
    () => undefined,
  );
}
