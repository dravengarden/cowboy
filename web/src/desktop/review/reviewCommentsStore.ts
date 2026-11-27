// Session-scoped staged inline Code Review comments (Antigravity / Claude Code
// Desktop style): reviewers annotate lines across multiple files without
// leaving Review, review/edit them in the Review tray or Prompt Companion Rail,
// and submit the entire batch to the Agent in one structured turn.

import { useSyncExternalStore } from "react";
import { appendDraftTextDurable } from "../../draftStore";
import { onSessionRemoved } from "../../removedSessionFence";
import type { CodeDiffScope } from "../../review/codeApi";
import { notify } from "../../store";

export type ReviewCommentSide = "new" | "old" | "patch";

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
  readonly scope?: CodeDiffScope | undefined;
  readonly side?: ReviewCommentSide | undefined;
  readonly commitOid?: string | undefined;
  readonly pullLabel?: string | undefined;
  readonly revision?: string | undefined;
  readonly createdAtMs: number;
}

const STORAGE_PREFIX = "cowboy.desktop.review-comments.v1:";
const VALID_KINDS = new Set(["diff", "source", "commit", "pr"]);
const VALID_SCOPES = new Set(["combined", "staged", "unstaged"]);
const VALID_SIDES = new Set(["new", "old", "patch"]);
const MAX_CACHED_SESSIONS = 32;
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

function isPositiveInt(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0;
}

function parseComment(item: unknown): StagedReviewComment | null {
  if (typeof item !== "object" || item === null) return null;
  const raw = item as Record<string, unknown>;
  if (
    typeof raw.id !== "string" || !raw.id ||
    typeof raw.path !== "string" || !raw.path ||
    typeof raw.body !== "string" || !raw.body.trim() ||
    typeof raw.codeSnippet !== "string" ||
    typeof raw.kind !== "string" || !VALID_KINDS.has(raw.kind) ||
    !isPositiveInt(raw.startLine) || !isPositiveInt(raw.endLine)
  ) return null;
  const startLine = Math.min(raw.startLine, raw.endLine);
  const endLine = Math.max(raw.startLine, raw.endLine);
  return {
    id: raw.id,
    path: raw.path,
    startLine,
    endLine,
    codeSnippet: raw.codeSnippet,
    kind: raw.kind as StagedReviewComment["kind"],
    body: raw.body,
    ...(typeof raw.root === "string" && raw.root ? { root: raw.root } : {}),
    ...(typeof raw.comparison === "string" && raw.comparison ? { comparison: raw.comparison } : {}),
    ...(typeof raw.scope === "string" && VALID_SCOPES.has(raw.scope) ? { scope: raw.scope as CodeDiffScope } : {}),
    ...(typeof raw.side === "string" && VALID_SIDES.has(raw.side) ? { side: raw.side as ReviewCommentSide } : {}),
    ...(typeof raw.commitOid === "string" && raw.commitOid ? { commitOid: raw.commitOid } : {}),
    ...(typeof raw.pullLabel === "string" && raw.pullLabel ? { pullLabel: raw.pullLabel } : {}),
    ...(typeof raw.revision === "string" && raw.revision ? { revision: raw.revision } : {}),
    createdAtMs: typeof raw.createdAtMs === "number" && Number.isFinite(raw.createdAtMs) ? raw.createdAtMs : 0,
  };
}

function setCached(sessionId: string, items: readonly StagedReviewComment[]): void {
  if (items.length === 0) {
    commentsBySession.delete(sessionId);
    return;
  }
  commentsBySession.delete(sessionId);
  commentsBySession.set(sessionId, items);
  while (commentsBySession.size > MAX_CACHED_SESSIONS) {
    const oldest = commentsBySession.keys().next().value;
    if (oldest === undefined || oldest === sessionId) break;
    commentsBySession.delete(oldest);
  }
}

function loadSession(sessionId: string): readonly StagedReviewComment[] {
  const cached = commentsBySession.get(sessionId);
  if (cached !== undefined) return cached;
  try {
    const raw = storage()?.getItem(`${STORAGE_PREFIX}${sessionId}`);
    if (!raw) return EMPTY;
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return EMPTY;
    const valid = parsed.map(parseComment).filter((c): c is StagedReviewComment => c !== null);
    if (valid.length === 0) return EMPTY;
    setCached(sessionId, valid);
    return valid;
  } catch {
    return EMPTY;
  }
}

function saveSession(sessionId: string, next: readonly StagedReviewComment[]): void {
  const normalized = next.length === 0 ? EMPTY : next;
  setCached(sessionId, normalized);
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

onSessionRemoved((removed) => {
  let changed = false;
  for (const id of removed) {
    if (commentsBySession.delete(id)) changed = true;
    try {
      storage()?.removeItem(`${STORAGE_PREFIX}${id}`);
    } catch {
      /* tolerant */
    }
  }
  if (changed) emit();
});

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

function longestBacktickRun(text: string): number {
  let longest = 0;
  for (const match of text.matchAll(/`+/g)) {
    if (match[0].length > longest) longest = match[0].length;
  }
  return longest;
}

/** CommonMark-safe inline code span that never breaks on backticks in paths. */
export function formatInlineCode(value: string): string {
  const delim = "`".repeat(longestBacktickRun(value) + 1);
  const needsPad = value.startsWith("`") || value.endsWith("`") || (value.length > 0 && value.trim().length === 0);
  return needsPad ? `${delim} ${value} ${delim}` : `${delim}${value}${delim}`;
}

/** CommonMark-safe fenced block delimiter longer than any backtick run inside. */
function formatCodeFence(text: string, language: string): string {
  const marker = "`".repeat(Math.max(3, longestBacktickRun(text) + 1));
  return `\n${marker}${language}\n${text}\n${marker}`;
}

export function formatCommentLocation(comment: Pick<StagedReviewComment, "path" | "startLine" | "endLine" | "side">): string {
  const range = comment.startLine === comment.endLine
    ? `L${String(comment.startLine)}`
    : `L${String(comment.startLine)}-${String(comment.endLine)}`;
  if (comment.side === "old") return `${comment.path} (removed ${range})`;
  if (comment.side === "patch") return `${comment.path} (patch row ${range})`;
  return `${comment.path}:${range}`;
}

function formatComparisonLabel(comparison: string): string {
  return comparison
    .replace(/^refs\/cowboy\/session-bases\/.*/, "session start")
    .replace(/^(committed:)?refs\/(heads|remotes)\//, "");
}

export function formatReviewCommentsBatch(comments: readonly StagedReviewComment[]): string {
  if (comments.length === 0) return "";
  const heading = comments.length === 1
    ? "Review note (1 item):"
    : `Review notes (${String(comments.length)} items):`;
  const blocks = comments.map((comment, idx) => {
    const location = formatInlineCode(formatCommentLocation(comment));
    const meta: string[] = [];
    if (comment.root) meta.push(`in ${formatInlineCode(comment.root)}`);
    if (comment.kind === "commit" && comment.commitOid) meta.push(`commit ${comment.commitOid.slice(0, 10)}`);
    if (comment.kind === "pr" && comment.pullLabel) meta.push(`pull request ${comment.pullLabel}`);
    if (comment.scope && comment.scope !== "combined") meta.push(comment.scope);
    if (comment.comparison) meta.push(`compared with ${formatComparisonLabel(comment.comparison)}`);
    if (comment.revision) meta.push(`rev ${comment.revision.slice(0, 10)}`);
    const suffix = meta.length > 0 ? ` (${meta.join(" · ")})` : "";
    const snippet = comment.codeSnippet.trim();
    const lang = comment.kind === "source" ? "" : "diff";
    const fence = snippet ? formatCodeFence(snippet, lang) : "";
    return `${String(idx + 1)}. ${location}${suffix}${fence}\n${comment.body}`;
  });
  return `${heading}\n\n${blocks.join("\n\n")}\n`;
}

export function submitReviewCommentsToPrompt(
  sessionId: string,
  onSubmitted?: () => "focused" | "deferred" | void,
): number {
  const items = loadSession(sessionId);
  if (items.length === 0) return 0;
  const transferredIds = new Set(items.map((item) => item.id));
  const formatted = formatReviewCommentsBatch(items);
  const durable = appendDraftTextDurable(sessionId, formatted);
  if (!durable) {
    notify(
      "Could not save review notes to Prompt (browser storage full); notes kept in Review.",
      "error",
    );
    return 0;
  }
  // Remove only the exact transferred IDs so concurrent additions survive.
  const remaining = loadSession(sessionId).filter((item) => !transferredIds.has(item.id));
  saveSession(sessionId, remaining);
  const outcome = onSubmitted?.();
  if (outcome === "deferred") {
    notify(
      items.length === 1
        ? "Added 1 review note to New Prompt (kept your current pending edit open)"
        : `Added ${String(items.length)} review notes to New Prompt (kept your current pending edit open)`,
      "info",
    );
  } else {
    notify(
      items.length === 1
        ? "Added 1 review note to Prompt"
        : `Added ${String(items.length)} review notes to Prompt`,
      "info",
    );
  }
  return items.length;
}
