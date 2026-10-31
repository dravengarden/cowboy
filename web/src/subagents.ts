// ACP subagents in a session transcript.
//
// The worker folds each subagent (a child ACP session the agent announced)
// into one complete `cowboy_subagent` snapshot in the parent timeline. A later
// snapshot of the same child, by `revision`, replaces an earlier one: the
// Controller keeps one durable row per child, while a live client may also
// hold intermediate snapshots. See src/acp_subagents.rs.

import { compactTokens } from "./modelUsage";
import type { AcpUpdate } from "./protocol";

export const SUBAGENT_UPDATE = "cowboy_subagent";

/** Desktop: stop the subagent whose row has focus (FOCUS.md "Conversation
 *  reader"). It opens the same confirmation as the row's Stop button. */
export const SUBAGENT_STOP_KEY = "X";

/** The enabled Stop of the subagent row that holds focus, if any. */
export function focusedSubagentStop(): HTMLButtonElement | null {
  const active = globalThis.document?.activeElement;
  if (!(active instanceof HTMLElement)) return null;
  const stop = active.closest<HTMLElement>("[data-subagent-row], [data-task-row]")
    ?.querySelector<HTMLButtonElement>("[data-subagent-stop], [data-task-stop]");
  return stop && !stop.disabled ? stop : null;
}

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
  /** The model the subagent runs on, as its adapter names it. */
  model: string | null;
  /** Tokens the subagent reported with its last idle state; 0 when none. */
  tokens: number;
  /** False once a working subagent stopped reporting: it stayed silent after
   *  the parent's turn ended, or its worker was replaced without announcing
   *  it again. Its outcome is unknown, not success or failure. */
  reporting: boolean;
  /** The Controller's receipt seq of this snapshot (I2), or null for rows
   *  from before the Controller stamped it. Clients order by it. */
  observedSeq: number | null;
  /** The worker incarnation that reported it (I3), when stamped. */
  incarnation: string | null;
  /** The Controller no longer has this row (cleared or retired): never
   *  presented as finished (I6). Set by reconciliation only. */
  unavailable?: boolean;
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
    model: text(update["model"]),
    tokens: count((update["usage"] as { totalTokens?: unknown } | null | undefined)?.totalTokens),
    reporting: update["reporting"] !== false,
    observedSeq: typeof update["observedSeq"] === "number" && Number.isFinite(update["observedSeq"])
      ? update["observedSeq"]
      : null,
    incarnation: text(update["incarnation"]),
  };
}

/** The worker that owned this subagent is gone, or a new one has not
 *  announced it: nobody reports on its work any more. A newer snapshot
 *  replaces this. */
export function orphanedSubagent(subagent: Subagent): Subagent {
  return isWorking(subagent)
    ? { ...subagent, reporting: false, cancellable: false }
    : subagent;
}

export function isWorking(subagent: Pick<Subagent, "state">): boolean {
  return subagent.state === "running" || subagent.state === "requires_action";
}

export type SubagentTone = "working" | "waiting" | "done" | "failed" | "stopped" | "unknown";

/** Presentation grouping only (docs/background-tasks.md §3.2), never a state
 *  machine: the raw `state`/`stopReason` stay in the detail view. */
export type DisplayGroup =
  | "active"
  | "waiting"
  | "needs_you"
  | "stopping"
  | "done"
  | "failed"
  | "stopped"
  | "ended"
  | "not_reporting"
  | "unavailable";

export interface SubagentStatus {
  label: string;
  tone: SubagentTone;
  group: DisplayGroup;
}

/** Groups whose chip states an observed outcome. */
export function terminalGroup(group: DisplayGroup): boolean {
  return group === "done" || group === "failed" || group === "stopped" || group === "ended";
}

export interface SubagentStatusContext {
  /** Current permission/elicitation requests this child raised that this
   *  client can answer (R4). */
  pendingRequests?: number;
  /** A Stop was accepted and the child has not reported idle yet. */
  stopping?: boolean;
}

export function subagentStatus(
  subagent: Subagent,
  context: SubagentStatusContext = {},
): SubagentStatus {
  if (subagent.unavailable) {
    return { label: "No longer available", tone: "unknown", group: "unavailable" };
  }
  // R2: losing contact is never an outcome.
  if (subagent.state !== "idle" && !subagent.reporting) {
    return { label: "Not reporting", tone: "unknown", group: "not_reporting" };
  }
  switch (subagent.state) {
    case "running":
      return context.stopping
        ? { label: "Stopping", tone: "working", group: "stopping" }
        : { label: "Working", tone: "working", group: "active" };
    case "requires_action":
      // R4: "Needs your answer" only with a request this client can answer.
      return (context.pendingRequests ?? 0) > 0
        ? { label: "Needs your answer", tone: "waiting", group: "needs_you" }
        : { label: "Waiting", tone: "waiting", group: "waiting" };
    case "idle":
      switch (subagent.stopReason) {
        case "end_turn":
          return subagent.error
            ? { label: "Failed", tone: "failed", group: "failed" }
            : { label: "Done", tone: "done", group: "done" };
        case "cancelled":
          return { label: "Stopped", tone: "stopped", group: "stopped" };
        case "error":
          return { label: "Failed", tone: "failed", group: "failed" };
        case "refusal":
          return { label: "Failed: declined", tone: "failed", group: "failed" };
        case "max_tokens":
        case "max_turn_requests":
          return { label: "Failed: hit its limit", tone: "failed", group: "failed" };
        case null:
          // R1: cessation was observed without an outcome (e.g. the adapter
          // restarted its worker). A reply alone is not proof of success.
          return subagent.error
            ? { label: "Failed", tone: "failed", group: "failed" }
            : { label: "Ended: outcome unreported", tone: "unknown", group: "ended" };
        default:
          return { label: `Ended: ${subagent.stopReason.replaceAll("_", " ")}`, tone: "unknown", group: "ended" };
      }
    default:
      // `unknown` (and any adapter-specific state) is not terminal.
      return { label: "Not reporting", tone: "unknown", group: "not_reporting" };
  }
}

/** The subagent's foreground work has ended (an `idle` snapshot). */
export function subagentTerminal(subagent: Pick<Subagent, "state">): boolean {
  return subagent.state === "idle";
}

function plural(value: number, one: string, many: string): string {
  return `${String(value)} ${value === 1 ? one : many}`;
}

/** "Haiku 4.5 · 3 tool calls · 2 messages · 12k tokens", or "" before any
 *  activity. */
export function subagentCounts(subagent: Subagent): string {
  const parts: string[] = [];
  if (subagent.model) parts.push(subagent.model);
  if (subagent.toolCalls) parts.push(plural(subagent.toolCalls, "tool call", "tool calls"));
  if (subagent.messages) parts.push(plural(subagent.messages, "message", "messages"));
  if (subagent.tokens) parts.push(`${compactTokens(subagent.tokens)} tokens`);
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

/** Whether `next` is at least as new as `current` (I2): the Controller's
 *  receipt seq when both are stamped, else the producer revision. */
export function newerSubagent(
  current: Pick<Subagent, "revision" | "observedSeq">,
  next: Pick<Subagent, "revision" | "observedSeq">,
): boolean {
  if (current.observedSeq !== null && next.observedSeq !== null) {
    return next.observedSeq >= current.observedSeq;
  }
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
    model: next.model ?? current.model,
  };
}

/** The child session id a permission or elicitation request names, if a
 *  child raised it. */
export function requestingSubagentId(toolCall: unknown): string | null {
  if (!toolCall || typeof toolCall !== "object") return null;
  const cowboy = (toolCall as { _cowboy?: unknown })._cowboy;
  if (!cowboy || typeof cowboy !== "object") return null;
  const subagent = (cowboy as { subagent?: unknown }).subagent;
  if (!subagent || typeof subagent !== "object") return null;
  const id = (subagent as { sessionId?: unknown }).sessionId;
  return typeof id === "string" && id ? id : null;
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
