import type { DeliveryOrigin, DeliveryStatus } from "./localFirstDelivery.ts";

/** The subset of a parked row the queue mutators need to move or hide. */
export interface QueueItem {
  readonly id: string;
  readonly text: string;
  readonly cmid?: string;
  readonly status?: DeliveryStatus;
  readonly origin?: DeliveryOrigin;
  readonly schedule?: unknown;
}

export interface QueueValue<R extends QueueItem = QueueItem> {
  readonly queue: readonly R[];
  readonly drafts: readonly R[];
  readonly inFlight: readonly R[];
}

export interface DraftActivation<R extends QueueItem = QueueItem> {
  id: string;
  /** A local draft has no server id yet. Retain its creation identity until
   * the authoritative echo supplies one; sending must move, never copy it. */
  sourceCmid?: string;
  row?: R;
  destination?: "queue" | "transcript";
}

export function draftActivationSourceId(
  activation: DraftActivation,
  drafts: readonly QueueItem[],
): string | null {
  return activation.sourceCmid === undefined
    ? activation.id
    : drafts.find((row) => row.cmid === activation.sourceCmid)?.id ?? null;
}

export function emptyQueueValue<R extends QueueItem>(): QueueValue<R> {
  return { queue: [], drafts: [], inFlight: [] };
}

export function removeById<R extends QueueItem>(
  rows: readonly R[],
  id: string,
): readonly R[] {
  return rows.some((row) => row.id === id) ? rows.filter((row) => row.id !== id) : rows;
}

export function appendUnique<R extends QueueItem>(
  rows: readonly R[],
  row: R,
): readonly R[] {
  if (
    rows.some((existing) =>
      existing.id === row.id ||
      (row.cmid !== undefined && existing.cmid === row.cmid)
    )
  ) {
    return rows;
  }
  return [...rows, row];
}

export function prependUnique<R extends QueueItem>(
  rows: readonly R[],
  row: R,
): readonly R[] {
  const without = rows.filter((existing) =>
    existing.id !== row.id &&
    !(row.cmid !== undefined && existing.cmid === row.cmid)
  );
  return [row, ...without];
}

export function replaceById<R extends QueueItem>(
  rows: readonly R[],
  id: string,
  row: R,
): readonly R[] {
  return rows.some((candidate) => candidate.id === id)
    ? rows.map((candidate) => candidate.id === id ? row : candidate)
    : rows;
}

export const queueMutators = {
  addDraft: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { row: R },
  ): QueueValue<R> => ({ ...value, drafts: appendUnique(value.drafts, args.row) }),
  scheduleDraft: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { row: R },
  ): QueueValue<R> => ({ ...value, drafts: appendUnique(value.drafts, args.row) }),
  rescheduleDraft: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { id: string; row: R },
  ): QueueValue<R> => ({
    ...value,
    drafts: replaceById(value.drafts, args.id, args.row),
  }),
  editDraft: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { id: string; row: R },
  ): QueueValue<R> => ({
    ...value,
    drafts: replaceById(value.drafts, args.id, args.row),
  }),
  editQueue: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { id: string; row: R },
  ): QueueValue<R> => ({
    ...value,
    queue: replaceById(value.queue, args.id, args.row),
  }),
  removeDraft: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { id: string },
  ): QueueValue<R> => ({
    ...value,
    drafts: removeById(value.drafts, args.id),
  }),
  removeQueue: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { id: string },
  ): QueueValue<R> => ({
    ...value,
    queue: removeById(value.queue, args.id),
  }),
  unscheduleDraft: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { id: string; row: R },
  ): QueueValue<R> => ({
    ...value,
    drafts: replaceById(value.drafts, args.id, args.row),
  }),
  addQueue: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { row: R },
  ): QueueValue<R> => ({ ...value, queue: appendUnique(value.queue, args.row) }),
  submitPrompt: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { row: R },
  ): QueueValue<R> => ({
    ...value,
    inFlight: appendUnique(value.inFlight ?? [], args.row),
  }),
  forceQueue: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { row: R },
  ): QueueValue<R> => ({ ...value, queue: prependUnique(value.queue, args.row) }),
  frontQueue: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { row: R },
  ): QueueValue<R> => ({ ...value, queue: prependUnique(value.queue, args.row) }),
  activateDraft: <R extends QueueItem>(
    value: QueueValue<R>,
    args: DraftActivation<R>,
  ): QueueValue<R> => ({
    ...value,
    drafts: value.drafts.filter((row) =>
      row.id !== args.id &&
      !(args.sourceCmid !== undefined && row.cmid === args.sourceCmid)
    ),
    queue: args.row === undefined || args.destination === "transcript"
      ? value.queue
      : appendUnique(value.queue, args.row),
    inFlight: args.row !== undefined && args.destination === "transcript"
      ? appendUnique(value.inFlight ?? [], args.row)
      : value.inFlight,
  }),
  sendQueued: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { id: string; row: R },
  ): QueueValue<R> => ({
    ...value,
    queue: removeById(value.queue, args.id),
    inFlight: appendUnique(value.inFlight ?? [], args.row),
  }),
  forceQueued: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { id: string; row: R },
  ): QueueValue<R> => ({
    ...value,
    queue: removeById(value.queue, args.id),
    inFlight: appendUnique(value.inFlight ?? [], args.row),
  }),
  returnQueuedToDraft: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { id: string; row: R },
  ): QueueValue<R> => ({
    ...value,
    queue: removeById(value.queue, args.id),
    drafts: appendUnique(value.drafts, args.row),
  }),
};

export const QUEUE_TRANSITION_MUTATORS = new Set([
  "activateDraft",
  "sendQueued",
  "forceQueued",
  "returnQueuedToDraft",
]);

export function settledTransitionIds<R extends QueueItem>(
  pending: readonly { id: string; name: string; args: unknown }[],
  next: QueueValue<R>,
  sameContent: (left: R, right: R) => boolean = (left, right) =>
    left.text === right.text,
): string[] {
  const settled: string[] = [];
  for (const mutation of pending) {
    if (mutation.name === "activateDraft") {
      const args = mutation.args as DraftActivation;
      const sourcePending = pending.some((candidate) =>
        (args.sourceCmid !== undefined && candidate.id === args.sourceCmid) ||
        (candidate.name === "returnQueuedToDraft" &&
          (candidate.args as { id: string }).id === args.id &&
          next.queue.some((row) => row.id === args.id))
      );
      if (
        !sourcePending &&
        !next.drafts.some((row) =>
          row.id === args.id ||
          (args.sourceCmid !== undefined && row.cmid === args.sourceCmid)
        )
      ) settled.push(mutation.id);
    } else if (
      mutation.name === "sendQueued" ||
      mutation.name === "forceQueued" ||
      mutation.name === "returnQueuedToDraft"
    ) {
      const id = (mutation.args as { id: string }).id;
      if (!next.queue.some((row) => row.id === id)) settled.push(mutation.id);
    } else if (
      mutation.name === "editDraft" ||
      mutation.name === "editQueue" ||
      mutation.name === "rescheduleDraft" ||
      mutation.name === "unscheduleDraft"
    ) {
      const args = mutation.args as { id: string; row: R };
      const rows = mutation.name === "editQueue" ? next.queue : next.drafts;
      const authoritative = rows.find((row) => row.id === args.id);
      if (
        authoritative !== undefined &&
        sameContent(authoritative, args.row) &&
        ((mutation.name !== "rescheduleDraft" &&
            mutation.name !== "unscheduleDraft") ||
          JSON.stringify(authoritative.schedule) ===
            JSON.stringify(args.row.schedule))
      ) {
        settled.push(mutation.id);
      }
    } else if (mutation.name === "removeDraft" || mutation.name === "removeQueue") {
      const id = (mutation.args as { id: string }).id;
      const rows = mutation.name === "removeQueue" ? next.queue : next.drafts;
      if (!rows.some((row) => row.id === id)) settled.push(mutation.id);
    }
  }
  return settled;
}
