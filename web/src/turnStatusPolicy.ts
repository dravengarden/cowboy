import type { Status } from "./protocol";

export type TurnStatusKind = "paused" | "interrupted";

export interface TurnStatusSignals {
  status: Status;
  working: boolean;
  paused: boolean;
}

export function deriveTurnStatusKind({
  status,
  working,
  paused,
}: TurnStatusSignals): TurnStatusKind | null {
  if (status === "busy" || status === "starting" || working) return null;
  if (status === "crashed") return null;
  if (status === "interrupted") return "interrupted";
  if (paused) return "paused";
  return null;
}
