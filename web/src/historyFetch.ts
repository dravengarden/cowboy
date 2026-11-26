// Version-scoped immutable HTTP history-page fetcher shared by transcript
// scrollback and reconnect gap-fill.
import type { Envelope } from "./protocol";

export const HISTORY_FETCH_TIMEOUT_MS = 6_000;
export const SNAPSHOT_GAP_FILL_PAGES = 32;

export interface HistoryPage {
  events: Envelope[];
  next_before_seq: number | null;
  reached_start: boolean;
}

export function historyPageUrl(
  sessionId: string,
  beforeSeq: number,
  version: string,
): string {
  return `/api/history/${encodeURIComponent(sessionId)}?before_seq=${String(beforeSeq)}&v=${encodeURIComponent(version)}`;
}

export async function fetchHistoryPage(
  sessionId: string,
  beforeSeq: number,
  version: string,
  signal?: AbortSignal,
): Promise<HistoryPage | null> {
  try {
    const res = await fetch(
      historyPageUrl(sessionId, beforeSeq, version),
      signal === undefined ? undefined : { signal },
    );
    if (!res.ok) return null;
    return (await res.json()) as HistoryPage;
  } catch {
    return null;
  }
}

export async function fetchPreviousQuestionPage(
  sessionId: string,
  beforeSeq: number,
  version: string,
): Promise<HistoryPage | null> {
  try {
    const res = await fetch(
      `/api/history/${encodeURIComponent(sessionId)}?before_seq=${String(beforeSeq)}&question_page=true&v=${encodeURIComponent(version)}`,
    );
    if (!res.ok) return null;
    return (await res.json()) as HistoryPage;
  } catch {
    return null;
  }
}

export async function fetchQuestionPage(
  sessionId: string,
  pageId: string,
  version: string,
): Promise<{ events: Envelope[] } | null> {
  try {
    const response = await fetch(
      `/api/sessions/${encodeURIComponent(sessionId)}/question-pages/${encodeURIComponent(pageId)}?v=${encodeURIComponent(version)}`,
      // The newest page may still be growing. Never let WebKit reuse a partial
      // response captured between the user prompt and the agent answer.
      { cache: "no-store" },
    );
    if (!response.ok) return null;
    return (await response.json()) as { events: Envelope[] };
  } catch {
    return null;
  }
}
