import { latestAvailableCommands } from "../agentCommands";
import { isCompactingTail, latestCompactionCompletionSeq } from "../derive";
import type { AvailableCommand, Envelope } from "../protocol";

export interface DesktopTopBarTimelineSlice {
  availableCommands: AvailableCommand[];
  compactingTail: boolean;
  completionSeq: number;
}

const EMPTY_COMMANDS: AvailableCommand[] = [];
const EMPTY_SLICE: DesktopTopBarTimelineSlice = {
  availableCommands: EMPTY_COMMANDS,
  compactingTail: false,
  completionSeq: 0,
};
const CACHE = new WeakMap<Envelope[], DesktopTopBarTimelineSlice>();

export function desktopTopBarTimelineSlice(
  timeline: Envelope[] | undefined,
): DesktopTopBarTimelineSlice {
  if (!timeline || timeline.length === 0) return EMPTY_SLICE;
  const cached = CACHE.get(timeline);
  if (cached) return cached;
  const commands = latestAvailableCommands(timeline);
  const slice = {
    availableCommands: commands.length === 0 ? EMPTY_COMMANDS : commands,
    compactingTail: isCompactingTail(timeline),
    completionSeq: latestCompactionCompletionSeq(timeline),
  };
  CACHE.set(timeline, slice);
  return slice;
}

function sameCommand(a: AvailableCommand, b: AvailableCommand): boolean {
  return a.name === b.name && a.description === b.description;
}

export function sameDesktopTopBarTimelineSlice(
  a: DesktopTopBarTimelineSlice,
  b: DesktopTopBarTimelineSlice,
): boolean {
  return a === b ||
    (a.compactingTail === b.compactingTail &&
      a.completionSeq === b.completionSeq &&
      a.availableCommands.length === b.availableCommands.length &&
      a.availableCommands.every((command, index) =>
        sameCommand(command, b.availableCommands[index]!)
      ));
}
