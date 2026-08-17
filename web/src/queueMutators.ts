import type { DeliveryOrigin, DeliveryStatus } from "./localFirstDelivery.ts";

/** The subset of a parked row the queue mutators need to move or hide. */
export interface QueueItem {
  readonly id: string;
  readonly text: string;
  readonly cmid?: string;
  readonly status?: DeliveryStatus;
  readonly origin?: DeliveryOrigin;
}

export interface QueueValue<R extends QueueItem = QueueItem> {
  readonly queue: readonly R[];
  readonly drafts: readonly R[];
  readonly inFlight: readonly R[];
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

export const queueMutators = {
  addDraft: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { row: R },
  ): QueueValue<R> => ({ ...value, drafts: appendUnique(value.drafts, args.row) }),
  addQueue: <R extends QueueItem>(
    value: QueueValue<R>,
    args: { row: R },
  ): QueueValue<R> => ({ ...value, queue: appendUnique(value.queue, args.row) }),
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
    args: { id: string; row?: R },
  ): QueueValue<R> => ({
    ...value,
    drafts: removeById(value.drafts, args.id),
    queue: args.row === undefined ? value.queue : appendUnique(value.queue, args.row),
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
): string[] {
  const settled: string[] = [];
  for (const mutation of pending) {
    if (mutation.name === "activateDraft") {
      const id = (mutation.args as { id: string }).id;
      if (!next.drafts.some((row) => row.id === id)) settled.push(mutation.id);
    } else if (
      mutation.name === "sendQueued" ||
      mutation.name === "forceQueued" ||
      mutation.name === "returnQueuedToDraft"
    ) {
      const id = (mutation.args as { id: string }).id;
      if (!next.queue.some((row) => row.id === id)) settled.push(mutation.id);
    }
  }
  return settled;
}
