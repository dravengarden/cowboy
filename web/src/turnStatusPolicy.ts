import type { Status } from "./protocol";

export type TurnStatusKind =
  | "awaiting"
  | "paused"
  | "done"
  | "interrupted"
  | "error";

export interface TurnStatusSignals {
  status: Status;
  working: boolean;
  judging: boolean;
  awaitingUser: boolean;
  done: boolean;
  paused: boolean;
}

/**
 * Resolve only the settled/actionable status that belongs beside Composer.
 *
 * Judge progress is deliberately absent: it is transient turn activity and is
 * rendered at the live Transcript tail. Returning null while it runs also
 * suppresses the provisional awaiting flag until the verdict arrives.
 */
export function deriveTurnStatusKind({
  status,
  working,
  judging,
  awaitingUser,
  done,
  paused,
}: TurnStatusSignals): TurnStatusKind | null {
  if (status === "busy" || status === "starting") return null;
  if (working) return null;
  if (status === "crashed") return "error";
  if (status === "interrupted") return "interrupted";
  if (judging) return null;
  if (awaitingUser) return "awaiting";
  if (paused) return "paused";
  if (done) return "done";
  return null;
}
