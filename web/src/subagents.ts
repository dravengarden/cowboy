// ACP subagents in a session transcript.
//
// The worker folds each subagent (a child ACP session the agent announced)
// into one complete `cowboy_subagent` snapshot in the parent timeline. A later
// snapshot of the same child, by `revision`, replaces an earlier one: the
// Controller keeps one durable row per child, while a live client may also
// hold intermediate snapshots. See src/acp_subagents.rs.

import type { AcpUpdate } from "./protocol";

export const SUBAGENT_UPDATE = "cowboy_subagent";

export interface SubagentActivity {
  kind: "message" | "tool";
  /** Tool call id; absent for a message. */
  id?: string;
  /** Tool title or message text (bounded by the worker). */
  text: string;
  /** Tool status; "" for a message. */
  status: string;
  toolKind?: string;
}

export interface Subagent {
  /** The child ACP session id. */
  id: string;
  revision: number;
  /** The subagent that launched this one, or null for a direct child. */
  parentId: string | null;
  title: string;
  /** The adapter has not named it (yet); `title` is a placeholder. */
  untitled: boolean;
  description: string | null;
  /** running | idle | requires_action | unknown, or an adapter's own state. */
  state: string;
  stopReason: string | null;
  error: string | null;
  /** The adapter grants `cancel` now and the subagent is working. */
  cancellable: boolean;
  prompt: string | null;
  result: string | null;
  toolCalls: number;
  messages: number;
  activity: SubagentActivity[];
  omittedActivity: number;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function count(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? Math.floor(value)
    : 0;
}

function activityOf(value: unknown): SubagentActivity | null {
  if (!value || typeof value !== "object") return null;
  const entry = value as Record<string, unknown>;
  if (entry["kind"] === "message") {
    const body = text(entry["text"]);
    return body === null ? null : { kind: "message", text: body, status: "" };
  }
  if (entry["kind"] === "tool") {
    const id = typeof entry["id"] === "string" ? entry["id"] : "";
    const toolKind = text(entry["toolKind"]);
    return {
      kind: "tool",
      ...(id ? { id } : {}),
      text: text(entry["title"]) ?? (id || "Tool call"),
      status: text(entry["status"]) ?? "pending",
      ...(toolKind ? { toolKind } : {}),
    };
  }
  return null;
}

/** The subagent a `cowboy_subagent` update describes, or null. */
export function subagentOf(update: AcpUpdate): Subagent | null {
  if (update.sessionUpdate !== SUBAGENT_UPDATE) return null;
  const id = update["sessionId"];
  if (typeof id !== "string" || !id) return null;
  const activity = Array.isArray(update["activity"])
    ? update["activity"].map(activityOf).filter((entry) => entry !== null)
    : [];
  const title = text(update.title);
  const description = text(update["description"]);
  return {
    id,
    revision: count(update["revision"]),
    parentId: text(update["parentSessionId"]),
    title: title ?? description ?? "Subagent",
    untitled: title === null && description === null,
    description: description && description !== title ? description : null,
    state: text(update["state"]) ?? "unknown",
    stopReason: text(update["stopReason"]),
    error: text(update["error"]),
    cancellable: update["cancellable"] === true,
    prompt: text(update["prompt"]),
    result: text(update["result"]),
    toolCalls: count(update["toolCalls"]),
    messages: count(update["messages"]),
    activity,
    omittedActivity: count(update["omittedActivity"]),
  };
}

/** The worker that owned this subagent is gone: its work stopped with it. */
export function orphanedSubagent(subagent: Subagent): Subagent {
  return isWorking(subagent)
    ? { ...subagent, state: "unknown", cancellable: false }
    : subagent;
}

export function isWorking(subagent: Pick<Subagent, "state">): boolean {
  return subagent.state === "running" || subagent.state === "requires_action";
}

export type SubagentTone = "working" | "waiting" | "done" | "failed" | "stopped" | "unknown";

export interface SubagentStatus {
  label: string;
  tone: SubagentTone;
}

export function subagentStatus(subagent: Subagent): SubagentStatus {
  switch (subagent.state) {
    case "running":
      return { label: "Working", tone: "working" };
    case "requires_action":
      return { label: "Needs your answer", tone: "waiting" };
    case "idle":
      switch (subagent.stopReason) {
        case "cancelled":
          return { label: "Stopped", tone: "stopped" };
        case "error":
          return { label: "Failed", tone: "failed" };
        case "refusal":
          return { label: "Declined", tone: "failed" };
        case "max_tokens":
        case "max_turn_requests":
          return { label: "Hit its limit", tone: "failed" };
        default:
          return subagent.error
            ? { label: "Failed", tone: "failed" }
            : { label: "Done", tone: "done" };
      }
    default:
      return { label: "Ended", tone: "unknown" };
  }
}

function plural(value: number, one: string, many: string): string {
  return `${String(value)} ${value === 1 ? one : many}`;
}

/** "3 tool calls · 2 messages", or "" before any activity. */
export function subagentCounts(subagent: Subagent): string {
  const parts: string[] = [];
  if (subagent.toolCalls) parts.push(plural(subagent.toolCalls, "tool call", "tool calls"));
  if (subagent.messages) parts.push(plural(subagent.messages, "message", "messages"));
  return parts.join(" · ");
}

/** One line for the collapsed row: what the subagent is doing or did last. */
export function subagentPreview(subagent: Subagent): string {
  if (!isWorking(subagent) && subagent.result) return firstLine(subagent.result);
  const last = subagent.activity.at(-1);
  if (last) return firstLine(last.text);
  return subagent.prompt ? firstLine(subagent.prompt) : "";
}

function firstLine(value: string): string {
  const line = value.trim().split("\n").find((part) => part.trim()) ?? "";
  return line.length > 160 ? `${line.slice(0, 159)}…` : line;
}

/** The newer of two snapshots of one subagent. */
export function newerSubagent(current: Subagent, next: Subagent): boolean {
  return next.revision >= current.revision;
}

/** Apply a newer snapshot. A worker that only learned the child's state (it
 *  re-announced a subagent after a restore) keeps the labels and delegation
 *  an earlier worker reported. */
export function mergeSubagent(current: Subagent, next: Subagent): Subagent {
  if (!next.untitled || current.untitled) return next;
  return {
    ...next,
    title: current.title,
    untitled: false,
    description: next.description ?? current.description,
    prompt: next.prompt ?? current.prompt,
  };
}

/** The subagent a permission or elicitation request names, if a child raised it. */
export function requestingSubagent(toolCall: unknown): string | null {
  if (!toolCall || typeof toolCall !== "object") return null;
  const cowboy = (toolCall as { _cowboy?: unknown })._cowboy;
  if (!cowboy || typeof cowboy !== "object") return null;
  const subagent = (cowboy as { subagent?: unknown }).subagent;
  if (!subagent || typeof subagent !== "object") return null;
  const title = (subagent as { title?: unknown }).title;
  return typeof title === "string" && title.trim() ? title.trim() : "Subagent";
}
