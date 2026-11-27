// Tree projection, stable ordering, glanceable summaries, and Quote-to-Prompt
// formatting shared by Desktop and Mobile Background Tasks views.

import {
  type TaskSection,
  taskSection,
  type TaskSummary,
} from "../backgroundTasks";
import {
  callActive,
  callModelLabel,
  callStateLabel,
  elapsedLabel,
  type ManagedCallSummary,
} from "../managedCalls";
import { type Subagent, subagentCounts, terminalGroup } from "../subagents";

export interface TaskTabSummary {
  /** Session has retained task history or reported background activity. */
  hasTasks: boolean;
  totalCount: number;
  activeCount: number;
  notReportingCount: number;
  needsYouCount: number;
  failedUnseenCount: number;
  endedCount: number;
  /** Entry stays reachable while badges stay quiet. */
  quietHistory: boolean;
  extraActivity: boolean;
  lead: TaskSummary | null;
}

/** Separate entry lifetime (`hasTasks`) from status decoration (`active`/`needs`/`failed`). */
export function taskTabSummary(
  tasks: readonly TaskSummary[],
  backgroundActivity = 0,
): TaskTabSummary {
  const active = tasks.filter((task) => taskSection(task) === "active");
  const notReportingCount = active.filter((task) => task.group === "not_reporting").length;
  const activeCount = active.length - notReportingCount;
  const needsYouCount = tasks.filter((task) => task.group === "needs_you").length;
  const failedUnseenCount = tasks.filter((task) => task.attention === "failed_unseen").length;
  const endedCount = tasks.filter((task) => taskSection(task) === "ended").length;
  const workingSubagents = tasks.filter((task) =>
    task.source === "subagent" &&
    (task.group === "active" || task.group === "waiting" || task.group === "needs_you" || task.group === "stopping")
  ).length;
  const extraActivity = backgroundActivity > workingSubagents;
  const lead = tasks.find((task) => task.group === "needs_you") ??
    [...active].sort(compareTaskOrder)[0] ??
    tasks.find((task) => task.attention === "failed_unseen") ??
    [...tasks].sort(compareTaskOrder)[0] ??
    null;
  const hasTasks = tasks.length > 0 || extraActivity;
  const quietHistory = hasTasks &&
    activeCount === 0 &&
    notReportingCount === 0 &&
    needsYouCount === 0 &&
    failedUnseenCount === 0 &&
    !extraActivity;
  return {
    hasTasks,
    totalCount: tasks.length,
    activeCount,
    notReportingCount,
    needsYouCount,
    failedUnseenCount,
    endedCount,
    quietHistory,
    extraActivity,
    lead,
  };
}

function sourceRank(source: TaskSummary["source"]): number {
  switch (source) {
    case "approval":
      return 0;
    case "call":
      return 1;
    case "subagent":
      return 2;
  }
}

function orderPartition(task: TaskSummary): number {
  if (task.source === "approval") return 0;
  return task.anchorSeq !== null ? 1 : 2;
}

function unanchoredClock(task: TaskSummary): number {
  if (task.source === "call" && task.call) return task.call.created_at_ms;
  if (task.source === "subagent" && task.subagent) {
    return task.subagent.observedSeq ?? task.subagent.revision;
  }
  return task.sortAt;
}

/** Strict total-order comparator that never mixes incompatible clocks:
 *  Partition 0: pending approvals (sortAt desc).
 *  Partition 1: tasks anchored on the shared parent timeline (anchorSeq desc, then sourceRank).
 *  Partition 2: unanchored tasks (sourceRank first so each source stays in its own clock domain,
 *  then source-local timestamp/seq desc). Ties break by key. */
export function compareTaskOrder(a: TaskSummary, b: TaskSummary): number {
  const partDiff = orderPartition(a) - orderPartition(b);
  if (partDiff !== 0) return partDiff;
  if (a.anchorSeq !== null && b.anchorSeq !== null) {
    if (a.anchorSeq !== b.anchorSeq) return b.anchorSeq - a.anchorSeq;
    const rankDiff = sourceRank(a.source) - sourceRank(b.source);
    if (rankDiff !== 0) return rankDiff;
  } else {
    const rankDiff = sourceRank(a.source) - sourceRank(b.source);
    if (rankDiff !== 0) return rankDiff;
    const clockDiff = unanchoredClock(b) - unanchoredClock(a);
    if (clockDiff !== 0) return clockDiff;
  }
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

export interface TaskTreeNode {
  task: TaskSummary;
  children: TaskTreeNode[];
  /** Total descendants under this node. */
  descendantCount: number;
  /** Needs-you and unseen-failure descendants under this node. */
  descendantAttention: number;
  /** True when any descendant is active or needs attention. */
  hasActiveDescendant: boolean;
}

export interface TaskTreeSection {
  section: TaskSection;
  roots: TaskTreeNode[];
}

export interface FlatTaskRow {
  task: TaskSummary;
  depth: number;
  hasChildren: boolean;
  collapsed: boolean;
  descendantCount: number;
  descendantAttention: number;
  section: TaskSection;
}

const SECTION_ORDER: readonly TaskSection[] = ["needs_you", "active", "ended"];

function sectionPriority(section: TaskSection): number {
  return SECTION_ORDER.indexOf(section);
}

/** Build a real parentKey forest: each task renders once under its actual parent,
 *  guarded against cycles, with orphans hoisted to root. */
export function buildTaskForest(tasks: readonly TaskSummary[]): TaskTreeSection[] {
  const byKey = new Map(tasks.map((task) => [task.key, task]));
  const childMap = new Map<string, TaskSummary[]>();
  const roots: TaskSummary[] = [];

  const hasCycle = (startKey: string, parentKey: string): boolean => {
    const seen = new Set<string>([startKey]);
    let cursor: string | null = parentKey;
    while (cursor !== null) {
      if (seen.has(cursor)) return true;
      seen.add(cursor);
      cursor = byKey.get(cursor)?.parentKey ?? null;
    }
    return false;
  };

  for (const task of tasks) {
    const parentKey = task.parentKey;
    if (parentKey !== null && byKey.has(parentKey) && !hasCycle(task.key, parentKey)) {
      const list = childMap.get(parentKey) ?? [];
      list.push(task);
      childMap.set(parentKey, list);
    } else {
      roots.push(task);
    }
  }

  const buildNode = (task: TaskSummary): TaskTreeNode => {
    const rawChildren = [...(childMap.get(task.key) ?? [])].sort(compareTaskOrder);
    const children = rawChildren.map(buildNode);
    let descendantCount = 0;
    let descendantAttention = 0;
    let hasActiveDescendant = false;
    for (const child of children) {
      descendantCount += 1 + child.descendantCount;
      descendantAttention += (child.task.attention !== "none" ? 1 : 0) + child.descendantAttention;
      if (taskSection(child.task) !== "ended" || child.hasActiveDescendant) {
        hasActiveDescendant = true;
      }
    }
    return { task, children, descendantCount, descendantAttention, hasActiveDescendant };
  };

  const rootSection = (node: TaskTreeNode): TaskSection => {
    let best = taskSection(node.task);
    const visit = (current: TaskTreeNode): void => {
      for (const child of current.children) {
        const childSec = taskSection(child.task);
        if (sectionPriority(childSec) < sectionPriority(best)) {
          best = childSec;
        }
        visit(child);
      }
    };
    visit(node);
    return best;
  };

  const grouped = new Map<TaskSection, TaskTreeNode[]>([
    ["needs_you", []],
    ["active", []],
    ["ended", []],
  ]);

  for (const root of [...roots].sort(compareTaskOrder)) {
    const node = buildNode(root);
    const sec = rootSection(node);
    grouped.get(sec)!.push(node);
  }

  return SECTION_ORDER
    .map((section) => ({ section, roots: grouped.get(section)! }))
    .filter((entry) => entry.roots.length > 0);
}

/** Flatten visible rows for roster rendering, keyboard stepping, and sibling
 *  switching while honoring collapsed branches and the `Hide ended` filter. */
export function flattenTaskSections(
  sections: readonly TaskTreeSection[],
  options: {
    collapsedKeys?: ReadonlySet<string>;
    hideEnded?: boolean;
  } = {},
): FlatTaskRow[] {
  const collapsedKeys = options.collapsedKeys ?? new Set<string>();
  const hideEnded = options.hideEnded ?? false;
  const rows: FlatTaskRow[] = [];

  const includeNode = (node: TaskTreeNode): boolean => {
    if (!hideEnded) return true;
    return taskSection(node.task) !== "ended" || node.hasActiveDescendant;
  };

  const walk = (node: TaskTreeNode, depth: number, section: TaskSection): void => {
    if (!includeNode(node)) return;
    const visibleChildren = node.children.filter(includeNode);
    const collapsed = visibleChildren.length > 0 && collapsedKeys.has(node.task.key);
    rows.push({
      task: node.task,
      depth,
      hasChildren: visibleChildren.length > 0,
      collapsed,
      descendantCount: node.descendantCount,
      descendantAttention: node.descendantAttention,
      section,
    });
    if (!collapsed) {
      for (const child of visibleChildren) walk(child, depth + 1, section);
    }
  };

  for (const { section, roots } of sections) {
    for (const root of roots) walk(root, 0, section);
  }
  return rows;
}

function firstLine(text: string | null | undefined, maxChars = 110): string | null {
  if (!text) return null;
  const line = text
    .split(/\r?\n/)
    .map((part) => part.trim())
    .find((part) => part.length > 0);
  if (!line) return null;
  return line.length > maxChars ? `${line.slice(0, maxChars - 1)}…` : line;
}

function formatVerdict(verdict: string): string {
  const normalized = verdict.trim().toLowerCase();
  if (normalized === "approve" || normalized === "approved") return "Approved";
  if (normalized === "needs-attention" || normalized === "needs_attention") return "Needs attention";
  return verdict.replace(/[-_]+/g, " ").replace(/^\w/, (ch) => ch.toUpperCase());
}

/** Verdict + findings line for a completed or active Call summary. */
export function callVerdictLabel(call: ManagedCallSummary): string | null {
  const parts: string[] = [];
  if (call.verdict) parts.push(formatVerdict(call.verdict));
  if (call.finding_count !== undefined && call.finding_count !== null) {
    parts.push(`${String(call.finding_count)} finding${call.finding_count === 1 ? "" : "s"}`);
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

function subagentLiveLine(subagent: Subagent): string | null {
  for (let i = subagent.activity.length - 1; i >= 0; i -= 1) {
    const entry = subagent.activity[i]!;
    if (entry.kind === "tool") {
      if (entry.status === "in_progress" || entry.status === "pending") {
        return firstLine(entry.text);
      }
      const text = firstLine(entry.text);
      return text ? `Last: ${text}` : null;
    }
    const snippet = firstLine(entry.text);
    if (snippet) return snippet;
  }
  return firstLine(subagent.description) ?? subagentCounts(subagent);
}

/** Single glanceable second line answering "what is it doing / what did it find /
 *  what does it need?" without fabricating unseen activity or timings. */
export function taskHeadline(task: TaskSummary): string {
  if (task.source === "approval") {
    return task.subtitle || "Waiting for your approval";
  }
  if (task.source === "call" && task.call) {
    const call = task.call;
    if (callActive(call.state)) {
      const stage = call.state === "queued"
        ? "Queued"
        : call.state === "starting"
        ? "Starting"
        : call.state === "waiting_input"
        ? "Waiting for input"
        : call.state === "stopping"
        ? "Stopping"
        : "Working";
      const model = callModelLabel(call);
      return [stage, model].filter(Boolean).join(" · ");
    }
    if (call.state === "failed") {
      return firstLine(call.error?.detail) ??
        (call.error?.code ? `Failed: ${call.error.code.replaceAll("_", " ")}` : "Failed");
    }
    if (call.state === "cancelled") {
      return "Stopped";
    }
    const verdict = callVerdictLabel(call);
    if (verdict) return `Done · ${verdict}`;
    return call.has_result ? `Done · ${callModelLabel(call)}` : "Done · No result captured";
  }
  if (task.source === "subagent" && task.subagent) {
    const subagent = task.subagent;
    if (task.group === "needs_you") {
      return `${String(task.pendingRequests)} pending request${task.pendingRequests === 1 ? "" : "s"}`;
    }
    if (task.group === "active" || task.group === "waiting" || task.group === "stopping") {
      return subagentLiveLine(subagent) ?? callStateLabelFallback(task);
    }
    if (task.group === "failed") {
      return firstLine(subagent.error) ?? firstLine(subagent.result) ?? "Failed";
    }
    if (task.group === "unavailable") {
      return "Outcome unreported";
    }
    if (task.group === "not_reporting") {
      return subagentLiveLine(subagent) ?? "Not reporting";
    }
    if (task.group === "ended") {
      return firstLine(subagent.result) ?? "Outcome unreported";
    }
    return firstLine(subagent.result) ?? (task.group === "done" ? "No result captured" : task.subtitle || task.chip);
  }
  return task.subtitle || task.chip;
}

function callStateLabelFallback(task: TaskSummary): string {
  return task.call ? callStateLabel(task.call) : task.chip;
}

/** Trustworthy elapsed duration (only for Calls with recorded timestamps).
 *  Subagents with only sequence numbers never fabricate an elapsed timer. */
export function taskTimeLabel(task: TaskSummary, now = Date.now()): string | null {
  if (task.source !== "call" || !task.call) return null;
  const call = task.call;
  const start = call.state === "queued" ? call.created_at_ms : (call.started_at_ms ?? call.created_at_ms);
  const end = callActive(call.state) ? now : (call.finished_at_ms ?? call.updated_at_ms);
  if (!Number.isFinite(start) || !Number.isFinite(end)) return null;
  const label = elapsedLabel(start, end);
  return call.state === "queued" ? `queued ${label}` : label;
}

/** Compact worker label for switcher chips. */
export function taskPillLabel(task: TaskSummary): string {
  return task.title.replace(/^(Review|Analysis|Call):\s*/i, "").trim() || task.title;
}

/** Format a structured reference block for `Quote to Prompt` (reusing Review's
 *  `appendDraftText` interaction without auto-sending). */
export function formatTaskQuote(task: TaskSummary, excerpt?: string | null): string {
  const ref = task.source === "call" && task.call
    ? `call ${task.call.call_id}`
    : task.source === "subagent" && task.subagent
    ? `subagent ${task.subagent.id}`
    : task.key;
  const statusLine = task.source === "call" && task.call && callVerdictLabel(task.call)
    ? `${task.chip} · ${callVerdictLabel(task.call)}`
    : task.chip;
  const body = (excerpt?.trim() ||
    task.subagent?.result?.trim() ||
    task.subagent?.error?.trim() ||
    task.call?.error?.detail?.trim() ||
    taskHeadline(task)).trim();
  const clipped = body.length > 1600 ? `${body.slice(0, 1599)}…` : body;
  return `Regarding background task "${task.title}" (${ref}, ${statusLine}):\n> ${clipped.replaceAll("\n", "\n> ")}`;
}

/** True when a task has reached a terminal execution state and should lead with
 *  its outcome/result. Unseen failed tasks stay in the Needs you roster section
 *  for triage, yet are still execution-terminal here. */
export function isTaskEnded(task: TaskSummary): boolean {
  if (task.source === "approval") return false;
  if (task.source === "call" && task.call) return !callActive(task.call.state);
  return terminalGroup(task.group) || task.group === "unavailable";
}

export type ActivityDotTone = "warning" | "error" | "active" | null;

export interface MobileActivityState {
  visible: boolean;
  hasTasks: boolean;
  hasBranches: boolean;
  dotTone: ActivityDotTone;
  defaultTab: "tasks" | "branches";
  label: string;
}

export function resolveMobileActivityState(
  summary: TaskTabSummary,
  relationCount: number,
  showTasks: boolean,
): MobileActivityState {
  const hasTasks = showTasks && summary.hasTasks;
  const hasBranches = relationCount > 0;
  if (!hasTasks && !hasBranches) {
    return {
      visible: false,
      hasTasks: false,
      hasBranches: false,
      dotTone: null,
      defaultTab: "tasks",
      label: "",
    };
  }
  const dotTone: ActivityDotTone = !hasTasks
    ? null
    : summary.needsYouCount > 0
    ? "warning"
    : summary.failedUnseenCount > 0
    ? "error"
    : summary.activeCount > 0 || summary.extraActivity
    ? "active"
    : null;
  const defaultTab: "tasks" | "branches" = hasTasks && (dotTone !== null || !hasBranches)
    ? "tasks"
    : "branches";
  const parts: string[] = [];
  if (hasTasks) {
    if (summary.needsYouCount > 0) parts.push(`${String(summary.needsYouCount)} needs you`);
    if (summary.failedUnseenCount > 0) parts.push(`${String(summary.failedUnseenCount)} failed`);
    if (summary.activeCount > 0) parts.push(`${String(summary.activeCount)} active`);
    if (parts.length === 0 && summary.totalCount > 0) parts.push(`${String(summary.totalCount)} ended`);
  }
  if (hasBranches) {
    parts.push(`${String(relationCount)} ${relationCount === 1 ? "branch" : "branches"}`);
  }
  const title = hasTasks && hasBranches
    ? "Session activity"
    : hasBranches
    ? "Branches & lineage"
    : "Background tasks";
  return {
    visible: true,
    hasTasks,
    hasBranches,
    dotTone,
    defaultTab,
    label: parts.length > 0 ? `${title}: ${parts.join(", ")}` : title,
  };
}
