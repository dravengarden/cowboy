import { latestAvailableCommands } from "./agentCommands";
import {
  type CurrentPlan,
  isCompactingTail,
  latestCompactionCompletionSeq,
  latestPendingPermission,
  latestPlan,
  type PendingPermission,
} from "./derive";
import type {
  AvailableCommand,
  Envelope,
  PermissionOption,
  PlanEntry,
} from "./protocol";

/** The small portion of transcript state that can actually change Composer UI.
 * Streaming message/thought/tool text is deliberately absent: Transcript owns
 * those frames, while rerendering the editor tree for them only burns CPU. */
export interface ComposerTimelineSlice {
  plan: CurrentPlan | null;
  pendingPermission: PendingPermission | null;
  availableCommands: AvailableCommand[];
  compactingTail: boolean;
  completionSeq: number;
  contextClearedSeq: number;
}

const EMPTY_COMMANDS: AvailableCommand[] = [];
const EMPTY_SLICE: ComposerTimelineSlice = {
  plan: null,
  pendingPermission: null,
  availableCommands: EMPTY_COMMANDS,
  compactingTail: false,
  completionSeq: 0,
  contextClearedSeq: 0,
};
const CACHE = new WeakMap<Envelope[], ComposerTimelineSlice>();

export function composerTimelineSlice(
  timeline: Envelope[] | undefined,
): ComposerTimelineSlice {
  if (!timeline || timeline.length === 0) return EMPTY_SLICE;
  const cached = CACHE.get(timeline);
  if (cached) return cached;
  const commands = latestAvailableCommands(timeline);
  const slice: ComposerTimelineSlice = {
    plan: latestPlan(timeline),
    pendingPermission: latestPendingPermission(timeline),
    availableCommands: commands.length === 0 ? EMPTY_COMMANDS : commands,
    compactingTail: isCompactingTail(timeline),
    completionSeq: latestCompactionCompletionSeq(timeline),
    contextClearedSeq: latestContextClearedSeq(timeline),
  };
  CACHE.set(timeline, slice);
  return slice;
}

function samePlanEntry(a: PlanEntry, b: PlanEntry): boolean {
  return a.content === b.content && a.priority === b.priority &&
    a.status === b.status;
}

function samePlan(a: CurrentPlan | null, b: CurrentPlan | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.key === b.key &&
    a.supersededByUserTurn === b.supersededByUserTurn &&
    sameArray(a.entries, b.entries, samePlanEntry);
}

function samePermissionOption(
  a: PermissionOption,
  b: PermissionOption,
): boolean {
  return a.optionId === b.optionId && a.name === b.name && a.kind === b.kind;
}

function samePendingPermission(
  a: PendingPermission | null,
  b: PendingPermission | null,
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.requestId === b.requestId && a.title === b.title &&
    sameArray(a.options, b.options, samePermissionOption);
}

function sameCommand(a: AvailableCommand, b: AvailableCommand): boolean {
  return a.name === b.name && a.description === b.description;
}

function sameArray<T>(
  a: readonly T[],
  b: readonly T[],
  equal: (left: T, right: T) => boolean,
): boolean {
  return a === b ||
    (a.length === b.length &&
      a.every((value, index) => equal(value, b[index]!)));
}

/** Semantic equality for useStoreSelector. A successor timeline containing only
 * transcript churn keeps the prior slice reference and skips Composer render. */
export function sameComposerTimelineSlice(
  a: ComposerTimelineSlice,
  b: ComposerTimelineSlice,
): boolean {
  return a === b ||
    (a.compactingTail === b.compactingTail &&
      a.completionSeq === b.completionSeq &&
      a.contextClearedSeq === b.contextClearedSeq &&
      samePlan(a.plan, b.plan) &&
      samePendingPermission(a.pendingPermission, b.pendingPermission) &&
      sameArray(a.availableCommands, b.availableCommands, sameCommand));
}

function latestContextClearedSeq(timeline: readonly Envelope[]): number {
  for (let index = timeline.length - 1; index >= 0; index -= 1) {
    const event = timeline[index]!;
    if (
      event.kind === "update" &&
      event.update.sessionUpdate === "context_cleared"
    ) return event.seq;
  }
  return 0;
}
