// Session-scoped staged inline Code Review comments (Antigravity / Claude Code
// Desktop style): reviewers annotate lines across multiple files without
// leaving Review, review/edit them in the Review tray or Prompt Companion Rail,
// and submit the entire batch to the Agent in one structured turn.

import { useSyncExternalStore } from "react";
import { appendDraftText } from "../../draftStore";
import { notify } from "../../store";

export interface StagedReviewComment {
  readonly id: string;
  readonly path: string;
  readonly startLine: number;
  readonly endLine: number;
  readonly codeSnippet: string;
  readonly kind: "diff" | "source" | "commit" | "pr";
  readonly body: string;
  readonly root?: string | undefined;
  readonly comparison?: string | undefined;
  readonly createdAtMs: number;
}

const STORAGE_PREFIX = "cowboy.desktop.review-comments.v1:";
const EMPTY: readonly StagedReviewComment[] = [];
const commentsBySession = new Map<string, readonly StagedReviewComment[]>();
const listeners = new Set<() => void>();

function storage(): Storage | null {
  try {
    return globalThis.sessionStorage ?? null;
  } catch {
    return null;
  }
}

function emit(): void {
  for (const listener of listeners) listener();
}

function loadSession(sessionId: string): readonly StagedReviewComment[] {
  const cached = commentsBySession.get(sessionId);
  if (cached !== undefined) return cached;
  try {
    const raw = storage()?.getItem(`${STORAGE_PREFIX}${sessionId}`);
    if (!raw) {
      commentsBySession.set(sessionId, EMPTY);
      return EMPTY;
    }
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) {
      commentsBySession.set(sessionId, EMPTY);
      return EMPTY;
    }
    const valid = parsed.filter((item): item is StagedReviewComment =>
      typeof item === "object" &&
      item !== null &&
      typeof (item as StagedReviewComment).id === "string" &&
      typeof (item as StagedReviewComment).path === "string" &&
      typeof (item as StagedReviewComment).body === "string" &&
      typeof (item as StagedReviewComment).startLine === "number" &&
      typeof (item as StagedReviewComment).endLine === "number"
    );
    const result = valid.length === 0 ? EMPTY : valid;
    commentsBySession.set(sessionId, result);
    return result;
  } catch {
    commentsBySession.set(sessionId, EMPTY);
    return EMPTY;
  }
}

function saveSession(sessionId: string, next: readonly StagedReviewComment[]): void {
  const normalized = next.length === 0 ? EMPTY : next;
  commentsBySession.set(sessionId, normalized);
  try {
    const store = storage();
    if (store) {
      if (normalized.length === 0) store.removeItem(`${STORAGE_PREFIX}${sessionId}`);
      else store.setItem(`${STORAGE_PREFIX}${sessionId}`, JSON.stringify(normalized));
    }
  } catch {
    // Non-fatal: in-memory staged comments stay intact for the tab session.
  }
  emit();
}

export function reviewCommentsForSession(sessionId: string | undefined): readonly StagedReviewComment[] {
  if (!sessionId) return EMPTY;
  return loadSession(sessionId);
}

export function useReviewComments(sessionId: string | undefined): readonly StagedReviewComment[] {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return (): void => {
        listeners.delete(listener);
      };
    },
    () => reviewCommentsForSession(sessionId),
    () => EMPTY,
  );
}

export function addReviewComment(
  sessionId: string,
  input: Omit<StagedReviewComment, "id" | "createdAtMs">,
): StagedReviewComment {
  const body = input.body.trim();
  const created: StagedReviewComment = {
    ...input,
    body,
    id: `rc-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`,
    createdAtMs: Date.now(),
  };
  saveSession(sessionId, [...loadSession(sessionId), created]);
  return created;
}

export function updateReviewComment(sessionId: string, id: string, body: string): void {
  const trimmed = body.trim();
  const prev = loadSession(sessionId);
  if (!trimmed) {
    removeReviewComment(sessionId, id);
    return;
  }
  const next = prev.map((item) => item.id === id ? { ...item, body: trimmed } : item);
  saveSession(sessionId, next);
}

export function removeReviewComment(sessionId: string, id: string): void {
  const prev = loadSession(sessionId);
  const next = prev.filter((item) => item.id !== id);
  if (next.length !== prev.length) saveSession(sessionId, next);
}

export function clearReviewComments(sessionId: string): void {
  if (loadSession(sessionId).length > 0) saveSession(sessionId, EMPTY);
}

export function formatReviewCommentsBatch(comments: readonly StagedReviewComment[]): string {
  if (comments.length === 0) return "";
  const heading = comments.length === 1
    ? "Review note (1 item):"
    : `Review notes (${String(comments.length)} items):`;
  const blocks = comments.map((comment, idx) => {
    const loc = comment.startLine === comment.endLine
      ? `L${String(comment.startLine)}`
      : `L${String(comment.startLine)}-${String(comment.endLine)}`;
    const rootNote = comment.root ? ` (in \`${comment.root}\`)` : "";
    const snippet = comment.codeSnippet.trim();
    const fence = snippet ? `\n\`\`\`${comment.kind === "diff" ? "diff" : ""}\n${snippet}\n\`\`\`` : "";
    return `${String(idx + 1)}. \`${comment.path}:${loc}\`${rootNote}${fence}\n${comment.body}`;
  });
  return `${heading}\n\n${blocks.join("\n\n")}\n`;
}

export function submitReviewCommentsToPrompt(
  sessionId: string,
  onSubmitted?: () => void,
): number {
  const items = loadSession(sessionId);
  if (items.length === 0) return 0;
  const formatted = formatReviewCommentsBatch(items);
  appendDraftText(sessionId, formatted);
  clearReviewComments(sessionId);
  notify(
    items.length === 1
      ? "Added 1 review note to Prompt"
      : `Added ${String(items.length)} review notes to Prompt`,
    "info",
  );
  onSubmitted?.();
  return items.length;
}
