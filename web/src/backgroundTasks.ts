// Background tasks: one presentation for ACP subagents and managed Calls
// (docs/background-tasks.md). This module owns
//   - the reconciliation authority: what the Controller says about each
//     subagent a device caches (bootstrap `subagent_state`, lookup), so a row
//     whose terminal snapshot was missed offline never stays "Working";
//   - `TaskSummary`, a small view model with one adapter per source. It is
//     never persisted and is not a state machine: each source keeps its own
//     fidelity in its existing detail view;
//   - device-local presentation state (failure acknowledgement, the legacy
//     Calls dock rollback switch).
//
// It must not import store.ts: the store feeds it.

import { persisted, useStore } from "@cowboy/state-store";
import { useSyncExternalStore } from "react";
import type { CallApproval } from "./callApproval";
import { callApprovalTitle } from "./callApproval";
import { derive, type PendingPermission, type RenderItem } from "./derive";
import {
  callActive,
  callModelLabel,
  callStateLabel,
  callTitle,
  elapsedLabel,
  type ManagedCallSummary,
  providerLabel,
} from "./managedCalls";
import { reportClientLog } from "./observability";
import type { Envelope } from "./protocol";
import {
  type DisplayGroup,
  isWorking,
  orphanedSubagent,
  newerSubagent,
  type Subagent,
  subagentCounts,
  subagentOf,
  subagentStatus,
  subagentTerminal,
  terminalGroup,
} from "./subagents";

// ---------------------------------------------------------------------------
// Wire

export interface SubagentStateItem {
  anchor_seq: number;
  update: Record<string, unknown>;
}

/** `subagent_state` of the focused bootstrap (Controller I5). */
export interface SubagentStateWire {
  session_id: string;
  epoch: string;
  watermark_seq: number;
  complete: boolean;
  incarnation: string | null;
  items: SubagentStateItem[];
}

interface LookupItem {
  id: string;
  status: "found" | "not_found";
  anchor_seq?: number;
  update?: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Reconciliation authority

export type ReconciledBy = "live" | "bootstrap" | "lookup" | "fallback" | "not_found";

export interface AuthorityEntry {
  subagent: Subagent;
  anchorSeq: number | null;
  reconciledBy: ReconciledBy;
  reconciledAt: number;
}

export interface SessionAuthority {
  /** The Controller sent `subagent_state`; false means fallback rendering. */
  supported: boolean;
  epoch: string | null;
  complete: boolean;
  /** The worker incarnation reporting for the session now. */
  incarnation: string | null;
  watermarkSeq: number;
  entries: ReadonlyMap<string, AuthorityEntry>;
  /** Ids the Controller no longer has a row for (cleared or retired). */
  notFound: ReadonlySet<string>;
  reconciledAt: number;
}

const authorities = new Map<string, SessionAuthority>();
// Advances on every bootstrap and on Clear or deletion, so a reconciliation
// still awaiting its lookups cannot publish over what replaced it.
const generations = new Map<string, number>();
const lookupsInFlight = new Map<string, Set<string>>();
const authorityListeners = new Set<() => void>();

function emitAuthority(): void {
  for (const listener of authorityListeners) listener();
}

function subscribeAuthority(listener: () => void): () => void {
  authorityListeners.add(listener);
  return (): void => {
    authorityListeners.delete(listener);
  };
}

export function sessionAuthority(sessionId: string): SessionAuthority | undefined {
  return authorities.get(sessionId);
}

export function useSessionAuthority(sessionId: string | null): SessionAuthority | undefined {
  return useSyncExternalStore(
    subscribeAuthority,
    () => (sessionId ? authorities.get(sessionId) : undefined),
    () => undefined,
  );
}

/** Forget a session's authority (Clear, deletion, sign-out). */
export function forgetSessionAuthority(sessionId: string): void {
  advanceGeneration(sessionId);
  if (authorities.delete(sessionId)) emitAuthority();
}

function advanceGeneration(sessionId: string): number {
  const generation = (generations.get(sessionId) ?? 0) + 1;
  generations.set(sessionId, generation);
  return generation;
}

/** The highest seq a cached timeline holds; sent as `since_seq`. */
export function timelineHighWater(timeline: readonly Envelope[] | undefined): number | null {
  const last = timeline?.at(-1);
  return last === undefined ? null : last.seq;
}

/** Every subagent a timeline renders, by child id. */
function timelineSubagents(timeline: readonly Envelope[]): Map<string, Subagent> {
  const subagents = new Map<string, Subagent>();
  for (const item of derive(timeline as Envelope[])) {
    if (item.kind === "subagent") subagents.set(item.subagent.id, item.subagent);
  }
  return subagents;
}

export type LookupTransport = (sessionId: string, ids: string[]) => Promise<LookupItem[]>;

export const LOOKUP_PAGE = 200;

async function defaultLookup(sessionId: string, ids: string[]): Promise<LookupItem[]> {
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(sessionId)}/subagents/lookup`,
    {
      method: "POST",
      cache: "no-store",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ids }),
    },
  );
  if (!response.ok) throw new Error(`subagent lookup failed (${String(response.status)})`);
  const body = await response.json() as { items?: LookupItem[] };
  return Array.isArray(body.items) ? body.items : [];
}

let lookupTransport: LookupTransport = defaultLookup;

/** Fixtures substitute the Controller lookup. */
export function setSubagentLookupTransport(transport: LookupTransport | null): void {
  lookupTransport = transport ?? defaultLookup;
}

function subagentFromItem(update: Record<string, unknown> | undefined): Subagent | null {
  if (!update) return null;
  return subagentOf({ sessionUpdate: "cowboy_subagent", ...update } as never);
}

export interface ReconcileReport {
  checked: number;
  changed: number;
  notFound: number;
  oldestStaleAgeMs: number;
  reconciledBy: ReconciledBy;
}

/**
 * Apply a focused bootstrap's `subagent_state` and resolve every cached
 * subagent the device still renders as non-terminal that the bootstrap did
 * not cover (I6). `state === undefined` means a Controller without the
 * contract: the cached rows render as-is (I7, fallback).
 */
export async function reconcileSubagents(
  sessionId: string,
  state: SubagentStateWire | undefined,
  timeline: readonly Envelope[],
  now: () => number = Date.now,
): Promise<ReconcileReport> {
  const generation = advanceGeneration(sessionId);
  const cached = timelineSubagents(timeline);
  if (state === undefined || state === null) {
    const entries = new Map<string, AuthorityEntry>();
    for (const subagent of cached.values()) {
      entries.set(subagent.id, {
        subagent,
        anchorSeq: null,
        reconciledBy: "fallback",
        reconciledAt: now(),
      });
    }
    authorities.set(sessionId, {
      supported: false,
      epoch: null,
      complete: false,
      incarnation: null,
      watermarkSeq: timelineHighWater(timeline) ?? 0,
      entries,
      notFound: new Set(),
      reconciledAt: now(),
    });
    emitAuthority();
    return { checked: 0, changed: 0, notFound: 0, oldestStaleAgeMs: 0, reconciledBy: "fallback" };
  }
  // Every bootstrap replaces the previous authority, so a new Controller
  // epoch (or a Clear) can never merge into an older ordering.
  const entries = new Map<string, AuthorityEntry>();
  const notFound = new Set<string>();
  const reconciledAt = now();
  let changed = 0;
  let oldestStaleAgeMs = 0;
  const covered = new Set<string>();
  const apply = (
    subagent: Subagent,
    anchorSeq: number | null,
    reconciledBy: ReconciledBy,
  ): void => {
    covered.add(subagent.id);
    notFound.delete(subagent.id);
    const before = cached.get(subagent.id);
    if (before && !subagentTerminal(before) && subagentTerminal(subagent)) {
      changed += 1;
      // The cached row was last observed at its own revision (producer ms).
      if (before.revision > 0 && before.revision < reconciledAt) {
        oldestStaleAgeMs = Math.max(oldestStaleAgeMs, reconciledAt - before.revision);
      }
    }
    const current = entries.get(subagent.id);
    if (current && !newerSubagent(current.subagent, subagent)) return;
    entries.set(subagent.id, { subagent, anchorSeq, reconciledBy, reconciledAt });
  };
  for (const item of state.items) {
    const subagent = subagentFromItem(item.update);
    if (subagent) apply(subagent, item.anchor_seq ?? null, "bootstrap");
  }
  const unresolved = [...cached.values()]
    .filter((subagent) => !subagentTerminal(subagent) && !covered.has(subagent.id))
    .map((subagent) => subagent.id);
  let lookupFailed = false;
  for (let start = 0; start < unresolved.length; start += LOOKUP_PAGE) {
    const page = unresolved.slice(start, start + LOOKUP_PAGE);
    let answers: LookupItem[];
    try {
      answers = await lookupTransport(sessionId, page);
    } catch {
      // Unresolved ids keep their cached rows; the next bootstrap retries.
      lookupFailed = true;
      break;
    }
    if (generations.get(sessionId) !== generation) break;
    for (const answer of answers) {
      if (answer.status === "found") {
        const subagent = subagentFromItem(answer.update);
        if (subagent) apply(subagent, answer.anchor_seq ?? null, "lookup");
      } else if (answer.status === "not_found" && page.includes(answer.id)) {
        notFound.add(answer.id);
        entries.delete(answer.id);
      }
    }
  }
  if (generations.get(sessionId) !== generation) {
    return { checked: 0, changed: 0, notFound: 0, oldestStaleAgeMs: 0, reconciledBy: "bootstrap" };
  }
  const authority: SessionAuthority = {
    supported: true,
    epoch: state.epoch,
    complete: state.complete && !lookupFailed,
    incarnation: typeof state.incarnation === "string" ? state.incarnation : null,
    watermarkSeq: state.watermark_seq,
    entries,
    notFound,
    reconciledAt,
  };
  authorities.set(sessionId, authority);
  emitAuthority();
  const report: ReconcileReport = {
    checked: unresolved.length + state.items.length,
    changed,
    notFound: [...notFound].filter((id) => cached.has(id)).length,
    oldestStaleAgeMs,
    reconciledBy: unresolved.length > 0 ? "lookup" : "bootstrap",
  };
  reportClientLog("info", "task.reconcile", "Background tasks reconciled", {
    session_id: sessionId,
    checked: report.checked,
    changed: report.changed,
    not_found: report.notFound,
    oldest_stale_age_ms: report.oldestStaleAgeMs,
    complete: authority.complete,
  });
  return report;
}

/** The ids a bootstrap's reconciliation has not answered: non-terminal rows a
 *  later history page or replica merge introduced. A row observed after the
 *  bootstrap's watermark is on the live stream and needs no lookup. */
function unreconciledSubagents(
  subagents: readonly Subagent[],
  authority: SessionAuthority,
): string[] {
  return subagents
    .filter((subagent) =>
      !subagentTerminal(subagent) &&
      !authority.entries.has(subagent.id) &&
      !authority.notFound.has(subagent.id) &&
      !observedLive(subagent, authority)
    )
    .map((subagent) => subagent.id);
}

function observedLive(subagent: Subagent, authority: SessionAuthority): boolean {
  return subagent.observedSeq !== null && subagent.observedSeq > authority.watermarkSeq;
}

/**
 * Resolve non-terminal rows that entered the timeline after the bootstrap
 * (I6): their terminal observation can predate `since_seq`, so neither the
 * bootstrap nor the immutable page that carried them will ever correct them.
 * Failed ids stay unanswered and are retried on the next timeline change.
 */
export async function reconcileLoadedSubagents(
  sessionId: string,
  subagents: readonly Subagent[],
  now: () => number = Date.now,
): Promise<void> {
  const authority = authorities.get(sessionId);
  if (!authority?.supported) return;
  const inFlight = lookupsInFlight.get(sessionId) ?? new Set<string>();
  const ids = unreconciledSubagents(subagents, authority).filter((id) => !inFlight.has(id));
  if (ids.length === 0) return;
  lookupsInFlight.set(sessionId, inFlight);
  for (const id of ids) inFlight.add(id);
  const generation = generations.get(sessionId);
  try {
    for (let start = 0; start < ids.length; start += LOOKUP_PAGE) {
      const page = ids.slice(start, start + LOOKUP_PAGE);
      const answers = await lookupTransport(sessionId, page);
      const current = authorities.get(sessionId);
      if (!current || generations.get(sessionId) !== generation) return;
      const entries = new Map(current.entries);
      const notFound = new Set(current.notFound);
      const reconciledAt = now();
      for (const answer of answers) {
        if (!page.includes(answer.id)) continue;
        const subagent = answer.status === "found" ? subagentFromItem(answer.update) : null;
        if (subagent) {
          entries.set(subagent.id, {
            subagent,
            anchorSeq: answer.anchor_seq ?? null,
            reconciledBy: "lookup",
            reconciledAt,
          });
        } else if (answer.status === "not_found") {
          notFound.add(answer.id);
        }
      }
      authorities.set(sessionId, { ...current, entries, notFound });
      emitAuthority();
    }
  } catch {
    // The rows keep their cached state; the next timeline change retries.
  } finally {
    for (const id of ids) inFlight.delete(id);
  }
}

/** A subagent row as the authority sees it now: the newer of the timeline's
 *  snapshot and the reconciled one. The fences apply to the result (I3): the
 *  timeline's own lifecycle boundary, then the current incarnation. */
export function authoritativeSubagent(
  subagent: Subagent,
  authority: SessionAuthority | undefined,
): { subagent: Subagent; reconciledBy: ReconciledBy; reconciledAt: number | null; anchorSeq: number | null } {
  if (!authority) {
    return { subagent, reconciledBy: "live", reconciledAt: null, anchorSeq: null };
  }
  // A `not_found` answer describes the bootstrap's moment. A snapshot observed
  // after it (the child was announced again, or the lookup raced its arrival)
  // supersedes that answer.
  if (authority.notFound.has(subagent.id) && !observedLive(subagent, authority)) {
    return {
      subagent: { ...subagent, unavailable: true, cancellable: false },
      reconciledBy: "not_found",
      reconciledAt: authority.reconciledAt,
      anchorSeq: null,
    };
  }
  const entry = authority.entries.get(subagent.id);
  let chosen = subagent;
  let reconciledBy: ReconciledBy = authority.supported ? "live" : "fallback";
  // An equal observation is the same snapshot: keep the timeline's row, which
  // carries any lifecycle fence applied since.
  if (entry && strictlyNewerSubagent(subagent, entry.subagent)) {
    chosen = { ...entry.subagent, title: entry.subagent.untitled ? subagent.title : entry.subagent.title };
    reconciledBy = entry.reconciledBy;
  }
  if (
    !subagent.reporting && isWorking(chosen) && chosen.incarnation === subagent.incarnation
  ) {
    // The timeline saw this worker stop; an older reconciled copy of its
    // snapshot does not bring it back.
    chosen = orphanedSubagent(chosen);
  }
  if (
    authority.incarnation !== null && chosen.incarnation !== null &&
    chosen.incarnation !== authority.incarnation && !observedLive(chosen, authority)
  ) {
    // Reported by a worker that is no longer the session's: its cached work
    // state and Stop grant are invalid until the new worker reports it. A
    // snapshot past the watermark is that report; the incarnation the
    // bootstrap named is the older fact.
    chosen = orphanedSubagent(chosen);
  }
  return {
    subagent: chosen,
    reconciledBy,
    reconciledAt: entry?.reconciledAt ?? authority.reconciledAt,
    anchorSeq: entry?.anchorSeq ?? null,
  };
}

function strictlyNewerSubagent(current: Subagent, next: Subagent): boolean {
  return newerSubagent(current, next) && !newerSubagent(next, current);
}

const APPLIED = new WeakMap<RenderItem[], { authority: SessionAuthority | undefined; items: RenderItem[] }>();

/** Replace each subagent row with its authoritative snapshot. Rows keep their
 *  transcript position (their anchor); only the content is repaired. */
export function applySubagentAuthority(
  items: RenderItem[],
  authority: SessionAuthority | undefined,
): RenderItem[] {
  if (!authority) return items;
  const cached = APPLIED.get(items);
  if (cached && cached.authority === authority) return cached.items;
  let changed = false;
  const next = items.map((item) => {
    if (item.kind !== "subagent") return item;
    const resolved = authoritativeSubagent(item.subagent, authority).subagent;
    if (resolved === item.subagent) return item;
    changed = true;
    return { ...item, subagent: resolved };
  });
  const result = changed ? next : items;
  APPLIED.set(items, { authority, items: result });
  return result;
}

/** Subagents the authority knows that the loaded transcript window does not
 *  render (their anchor is older than the window). The list still shows them. */
export function authorityOnlySubagents(
  items: readonly RenderItem[],
  authority: SessionAuthority | undefined,
): Subagent[] {
  if (!authority) return [];
  const rendered = new Set(items.flatMap((item) => item.kind === "subagent" ? [item.subagent.id] : []));
  return [...authority.entries.values()]
    .filter((entry) => !rendered.has(entry.subagent.id))
    .map((entry) => authoritativeSubagent(entry.subagent, authority).subagent);
}

// ---------------------------------------------------------------------------
// TaskSummary

export type TaskSource = "subagent" | "call" | "approval";

export interface TaskDiagnostics {
  source: TaskSource;
  entity: string;
  execution: string | null;
  incarnation: string | null;
  sourceRevision: string | null;
  observedSeq: number | null;
  anchorSeq: number | null;
  reconciledBy: ReconciledBy | null;
  reconciledAt: number | null;
}

export interface TaskSummary {
  /** "subagent:<child>" | "call:<call_id>" | "approval:<id>" */
  key: string;
  source: TaskSource;
  title: string;
  subtitle: string;
  group: DisplayGroup;
  /** Always shown; the terminal text once terminal. */
  chip: string;
  attention: "none" | "needs_you" | "failed_unseen";
  parentKey: string | null;
  /** Needs-you and unseen failures among descendants. */
  childAttention: number;
  /** Direct children, for the `(+N)` count. */
  childCount: number;
  /** A current Stop grant. */
  canStop: boolean;
  anchorSeq: number | null;
  /** Observation health beside the execution state (R3), e.g. "Reconnecting". */
  health: string | null;
  /** Pending requests this client can answer (R4). */
  pendingRequests: number;
  /** Ordering within a group: newest first. */
  sortAt: number;
  diag: TaskDiagnostics;
  subagent?: Subagent;
  call?: ManagedCallSummary;
  approval?: CallApproval;
}

export function subagentKey(child: string): string {
  return `subagent:${child}`;
}

export function callKey(callId: string): string {
  return `call:${callId}`;
}

export interface SubagentSummaryContext {
  pendingRequests?: number;
  stopping?: boolean;
  anchorSeq?: number | null;
  reconciledBy?: ReconciledBy | null;
  reconciledAt?: number | null;
  acknowledged?: (key: string, marker: string) => boolean;
}

/** The terminal marker an acknowledgement is bound to. A later failure of the
 *  same entity (a resumed child) is unseen again. */
function subagentMarker(subagent: Subagent): string {
  return String(subagent.observedSeq ?? subagent.revision);
}

export function subagentSummary(
  subagent: Subagent,
  context: SubagentSummaryContext = {},
): TaskSummary {
  const key = subagentKey(subagent.id);
  const pendingRequests = context.pendingRequests ?? 0;
  const status = subagent.unavailable
    ? { label: "No longer available", group: "unavailable" as const }
    : subagentStatus(subagent, {
      pendingRequests,
      ...(context.stopping === undefined ? {} : { stopping: context.stopping }),
    });
  const failedUnseen = status.group === "failed" &&
    !(context.acknowledged?.(key, subagentMarker(subagent)) ?? false);
  return {
    key,
    source: "subagent",
    title: subagent.title,
    subtitle: [subagent.description, subagentCounts(subagent)].filter(Boolean).join(" · "),
    group: status.group,
    chip: status.label,
    attention: status.group === "needs_you" ? "needs_you" : failedUnseen ? "failed_unseen" : "none",
    parentKey: subagent.parentId ? subagentKey(subagent.parentId) : null,
    childAttention: 0,
    childCount: 0,
    canStop: subagent.cancellable && !subagent.unavailable &&
      (status.group === "active" || status.group === "waiting" || status.group === "needs_you"),
    anchorSeq: context.anchorSeq ?? null,
    health: null,
    pendingRequests,
    sortAt: subagent.observedSeq ?? subagent.revision,
    diag: {
      source: "subagent",
      entity: subagent.id,
      execution: null,
      incarnation: subagent.incarnation,
      sourceRevision: String(subagent.revision),
      observedSeq: subagent.observedSeq,
      anchorSeq: context.anchorSeq ?? null,
      reconciledBy: context.reconciledBy ?? null,
      reconciledAt: context.reconciledAt ?? null,
    },
    subagent,
  };
}

export interface CallSummaryContext {
  anchorSeq?: number | null;
  /** The call's execution Machine is not connected now. */
  machineUnreachable?: boolean;
  /** The last ledger read failed; the shown state is the last observation. */
  stale?: boolean;
  acknowledged?: (key: string, marker: string) => boolean;
  now?: number;
}

function callGroupOf(call: ManagedCallSummary): DisplayGroup {
  switch (call.state) {
    case "queued":
    case "starting":
    case "running":
      return "active";
    case "waiting_input":
      return "waiting";
    case "stopping":
      return "stopping";
    case "completed":
      return "done";
    case "failed":
      return "failed";
    case "cancelled":
      return "stopped";
  }
}

function callChip(call: ManagedCallSummary): string {
  switch (call.state) {
    case "queued":
      return "Queued";
    case "starting":
      return "Starting";
    case "running":
      return "Working";
    case "waiting_input":
      return "Waiting";
    case "stopping":
      return "Stopping";
    case "completed":
      return "Done";
    case "failed":
      return call.error?.code ? `Failed: ${call.error.code.replaceAll("_", " ")}` : "Failed";
    case "cancelled":
      return "Stopped";
  }
}

export function callSummary(
  call: ManagedCallSummary,
  context: CallSummaryContext = {},
): TaskSummary {
  const key = callKey(call.call_id);
  const group = callGroupOf(call);
  const now = context.now ?? Date.now();
  const active = callActive(call.state);
  const model = callModelLabel(call);
  const elapsed = call.finished_at_ms && (call.started_at_ms ?? call.created_at_ms)
    ? elapsedLabel(call.started_at_ms ?? call.created_at_ms, call.finished_at_ms)
    : active
    ? elapsedLabel(call.started_at_ms ?? call.created_at_ms, now)
    : "";
  const failedUnseen = group === "failed" &&
    !(context.acknowledged?.(key, String(call.finished_at_ms ?? call.updated_at_ms)) ?? false);
  return {
    key,
    source: "call",
    title: callTitle(call),
    subtitle: [providerLabel(call.provider), model, elapsed].filter(Boolean).join(" · "),
    group,
    chip: callChip(call),
    attention: failedUnseen ? "failed_unseen" : "none",
    parentKey: null,
    childAttention: 0,
    childCount: 0,
    canStop: active && !call.cancel_requested && call.state !== "stopping",
    anchorSeq: context.anchorSeq ?? null,
    // R3: the ledger state is kept while its Machine is unreachable.
    health: active && (context.machineUnreachable || context.stale) ? "Reconnecting" : null,
    pendingRequests: 0,
    sortAt: call.created_at_ms,
    diag: {
      source: "call",
      entity: call.child_session_id,
      execution: call.call_id,
      incarnation: null,
      sourceRevision: callStateLabel(call),
      observedSeq: null,
      anchorSeq: context.anchorSeq ?? null,
      reconciledBy: null,
      reconciledAt: null,
    },
    call,
  };
}

/** A pending admission is its own attention item, not a Call (§5). */
export function approvalSummary(approval: CallApproval): TaskSummary {
  const id = approval.request_ids?.join(",") ?? String(approval.prompt_revision ?? 0);
  return {
    key: `approval:${id}`,
    source: "approval",
    title: callApprovalTitle(approval),
    subtitle: approval.items.map((item) => item.summary).filter(Boolean).slice(0, 2).join(" · "),
    group: "needs_you",
    chip: "Approve call",
    attention: "needs_you",
    parentKey: null,
    childAttention: 0,
    childCount: 0,
    canStop: false,
    anchorSeq: null,
    health: null,
    pendingRequests: approval.requests,
    sortAt: Number.MAX_SAFE_INTEGER,
    diag: {
      source: "approval",
      entity: id,
      execution: null,
      incarnation: null,
      sourceRevision: approval.prompt_revision === undefined ? null : String(approval.prompt_revision),
      observedSeq: null,
      anchorSeq: null,
      reconciledBy: null,
      reconciledAt: null,
    },
    approval,
  };
}

/** Roll nested attention into ancestors so `(+N)` cannot hide a blocked
 *  child, and count direct children. */
export function rollUpChildren(tasks: TaskSummary[]): TaskSummary[] {
  const byKey = new Map(tasks.map((task) => [task.key, { ...task }]));
  for (const task of byKey.values()) {
    let parent = task.parentKey ? byKey.get(task.parentKey) : undefined;
    if (parent) parent.childCount += 1;
    const attention = task.attention !== "none" ? 1 : 0;
    const seen = new Set<string>([task.key]);
    while (parent && attention > 0 && !seen.has(parent.key)) {
      seen.add(parent.key);
      parent.childAttention += attention;
      parent = parent.parentKey ? byKey.get(parent.parentKey) : undefined;
    }
  }
  return tasks.map((task) => byKey.get(task.key) ?? task);
}

export type TaskSection = "needs_you" | "active" | "ended";

export function taskSection(task: TaskSummary): TaskSection {
  if (task.group === "needs_you" || task.attention === "failed_unseen") return "needs_you";
  if (terminalGroup(task.group) || task.group === "unavailable") return "ended";
  return "active";
}

export const TASK_SECTION_LABEL: Record<TaskSection, string> = {
  needs_you: "Needs you",
  active: "Active",
  ended: "Ended",
};

/** Sections in display order with their tasks, newest first inside each. */
export function groupTasks(tasks: readonly TaskSummary[]): { section: TaskSection; tasks: TaskSummary[] }[] {
  const order: TaskSection[] = ["needs_you", "active", "ended"];
  return order
    .map((section) => ({
      section,
      tasks: tasks.filter((task) => taskSection(task) === section)
        .sort((a, b) => b.sortAt - a.sortAt || a.key.localeCompare(b.key)),
    }))
    .filter((entry) => entry.tasks.length > 0);
}

export interface TaskInputs {
  subagents: readonly { subagent: Subagent; anchorSeq: number | null; reconciledBy: ReconciledBy | null; reconciledAt: number | null }[];
  calls: readonly ManagedCallSummary[];
  callAnchors?: ReadonlyMap<string, number>;
  approval: CallApproval | null;
  pending: readonly PendingPermission[];
  stopping?: ReadonlySet<string>;
  unreachableMachines?: ReadonlySet<string>;
  callsStale?: boolean;
  acknowledged?: (key: string, marker: string) => boolean;
  now?: number;
}

export function buildTaskSummaries(inputs: TaskInputs): TaskSummary[] {
  const pendingByChild = new Map<string, number>();
  for (const request of inputs.pending) {
    if (request.subagentId) {
      pendingByChild.set(request.subagentId, (pendingByChild.get(request.subagentId) ?? 0) + 1);
    }
  }
  const tasks: TaskSummary[] = [];
  for (const entry of inputs.subagents) {
    tasks.push(subagentSummary(entry.subagent, {
      pendingRequests: pendingByChild.get(entry.subagent.id) ?? 0,
      stopping: inputs.stopping?.has(subagentKey(entry.subagent.id)) ?? false,
      anchorSeq: entry.anchorSeq,
      reconciledBy: entry.reconciledBy,
      reconciledAt: entry.reconciledAt,
      ...(inputs.acknowledged ? { acknowledged: inputs.acknowledged } : {}),
    }));
  }
  for (const call of inputs.calls) {
    const machine = call.runtime_machine_id ?? call.placement.machine_id;
    tasks.push(callSummary(call, {
      anchorSeq: inputs.callAnchors?.get(call.call_id) ?? null,
      machineUnreachable: inputs.unreachableMachines?.has(machine) ?? false,
      stale: inputs.callsStale ?? false,
      ...(inputs.acknowledged ? { acknowledged: inputs.acknowledged } : {}),
      ...(inputs.now === undefined ? {} : { now: inputs.now }),
    }));
  }
  if (inputs.approval) tasks.push(approvalSummary(inputs.approval));
  return rollUpChildren(tasks);
}

export interface BarSummary {
  working: number;
  /** Active rows whose source lost contact: never counted as working. */
  notReporting: number;
  needsYou: number;
  failed: number;
  /** The task the bar names: needs-you first, then the newest active one. */
  lead: TaskSummary | null;
  /** The legacy aggregate exceeds the working subagent rows (§6.3). */
  extraActivity: boolean;
  visible: boolean;
}

/** The one-line bar. It counts only TaskSummary items; the legacy
 *  `background_tasks` count (max(native, working subagents)) is never added,
 *  only shown as a neutral mark when it exceeds the working subagent rows. */
export function barSummary(tasks: readonly TaskSummary[], backgroundTasks = 0): BarSummary {
  const active = tasks.filter((task) => taskSection(task) === "active");
  const notReporting = active.filter((task) => task.group === "not_reporting").length;
  const working = active.length - notReporting;
  const needsYou = tasks.filter((task) => task.group === "needs_you").length;
  const failed = tasks.filter((task) => task.attention === "failed_unseen").length;
  const workingSubagents = tasks.filter((task) =>
    task.source === "subagent" && (task.group === "active" || task.group === "waiting" || task.group === "needs_you" || task.group === "stopping")
  ).length;
  const lead = tasks.find((task) => task.group === "needs_you") ??
    [...tasks].filter((task) => taskSection(task) === "active").sort((a, b) => b.sortAt - a.sortAt)[0] ??
    tasks.find((task) => task.attention === "failed_unseen") ??
    null;
  const extraActivity = backgroundTasks > workingSubagents;
  return {
    working,
    notReporting,
    needsYou,
    failed,
    lead,
    extraActivity,
    visible: active.length > 0 || needsYou > 0 || failed > 0 || extraActivity,
  };
}

/** The Call anchor rows of a transcript, by call id. */
export function callAnchorsOf(items: readonly RenderItem[]): Map<string, { key: string; seq: number; requestId: string; operation: string | null }> {
  const anchors = new Map<string, { key: string; seq: number; requestId: string; operation: string | null }>();
  for (const item of items) {
    if (item.kind === "call") {
      anchors.set(item.callId, {
        key: item.key,
        seq: Number(item.key),
        requestId: item.requestId,
        operation: item.origin.kind === "mcp" ? item.origin.operation : null,
      });
    }
  }
  return anchors;
}

/** The tool card that started a call through MCP (`cowboy_execute` with this
 *  operation). Only structured `rawInput` links; output text never does. */
export function originToolKey(items: readonly RenderItem[], requestId: string): string | null {
  if (!requestId) return null;
  for (const item of items) {
    if (item.kind !== "tool") continue;
    const input = item.rawInput && typeof item.rawInput === "object"
      ? item.rawInput as Record<string, unknown>
      : null;
    if (!input) continue;
    const action = input["action"];
    if (
      input["operation"] === requestId &&
      (action === "calls.start" || action === "review.start")
    ) return item.key;
  }
  return null;
}

// ---------------------------------------------------------------------------
// Device-local presentation state

// Phase 1 keeps failure acknowledgement on this device; per-account
// acknowledgement is Phase 2.
const acknowledgements = persisted<string[]>("cowboy:background-tasks:ack", [], {
  serialize: (value) => JSON.stringify(value.slice(-500)),
  deserialize: (raw) => {
    try {
      const value: unknown = JSON.parse(raw);
      return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : [];
    } catch {
      return [];
    }
  },
});

export function useAcknowledgements(): (key: string, marker: string) => boolean {
  const list = useStore(acknowledgements);
  const set = new Set(list);
  return (key, marker) => set.has(`${key}@${marker}`);
}

export function acknowledgeTask(task: TaskSummary): void {
  if (task.attention !== "failed_unseen") return;
  const marker = task.source === "subagent" && task.subagent
    ? String(task.subagent.observedSeq ?? task.subagent.revision)
    : task.call
    ? String(task.call.finished_at_ms ?? task.call.updated_at_ms)
    : "";
  const entry = `${task.key}@${marker}`;
  const current = acknowledgements.get();
  if (!current.includes(entry)) acknowledgements.set([...current, entry]);
}

/**
 * Rollback switch: `ui.calls_dock_legacy` restores the old Calls dock instead
 * of the background-tasks bar. Remove after two Web releases or 14 days,
 * whichever is longer — the deadline is recorded in docs/background-tasks.md.
 */
export const CALLS_DOCK_LEGACY_KEY = "cowboy:ui.calls_dock_legacy";
const callsDockLegacy = persisted(CALLS_DOCK_LEGACY_KEY, false, {
  serialize: (on) => (on ? "1" : "0"),
  deserialize: (raw) => raw === "1",
});

export function useCallsDockLegacy(): boolean {
  return useStore(callsDockLegacy);
}

export function setCallsDockLegacy(on: boolean): void {
  callsDockLegacy.set(on);
}

// ---------------------------------------------------------------------------
// Deep links

/** `#task=<key>` opens that task's detail. */
export function taskKeyFromHash(hash: string): string | null {
  const match = /(?:^#|&)task=([^&]+)/.exec(hash);
  if (!match?.[1]) return null;
  try {
    const key = decodeURIComponent(match[1]);
    return /^(subagent|call|approval):[^\s]{1,300}$/.test(key) ? key : null;
  } catch {
    return null;
  }
}

export function taskHash(key: string): string {
  return `#task=${encodeURIComponent(key)}`;
}
