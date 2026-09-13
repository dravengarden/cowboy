import type { Status } from "./protocol";

export type TranscriptRestoreKind = "hydrate" | "backfill" | "paused";

/** Conversation-shaped ghost turns. Enough rows to fill an iPad reading
 * surface; shorter phones clip the overflow behind the edge fade. */
export const CONVERSATION_SKELETON_TURNS: readonly {
  mine: boolean;
  lines: readonly string[];
}[] = [
  { mine: false, lines: ["88%", "74%", "46%"] },
  { mine: true, lines: ["52%"] },
  { mine: false, lines: ["92%", "81%", "63%", "38%"] },
  { mine: false, lines: ["71%", "54%"] },
  { mine: true, lines: ["36%"] },
  { mine: false, lines: ["86%", "69%", "44%"] },
  { mine: true, lines: ["48%"] },
  { mine: false, lines: ["90%", "77%", "51%"] },
  { mine: false, lines: ["68%", "41%"] },
  { mine: true, lines: ["42%"] },
  { mine: false, lines: ["84%", "62%"] },
  { mine: false, lines: ["93%", "70%", "47%"] },
];

export function transcriptRestoreCaption(
  kind: TranscriptRestoreKind,
  agent?: string,
): string {
  if (kind === "paused") return "Earlier messages";
  if (kind === "backfill") return "Loading earlier messages";
  const name = agent?.trim();
  return name ? `Restoring ${name} conversation` : "Restoring conversation";
}

/** Clean worker exits are session chrome (dormant), not a chat event. Crashes
 * and interrupted turns stay in the log because they explain a broken turn. */
export function shouldPaintTranscriptLifecycle(status: Status): boolean {
  return status === "crashed" || status === "interrupted";
}

export function transcriptLifecycleLabel(
  status: Status,
  detail: string | null,
  prettifyDetail: (detail: string) => string,
): string | null {
  if (status === "crashed") {
    return detail ? prettifyDetail(detail) : "Agent stopped unexpectedly";
  }
  if (status === "interrupted") {
    return "Last turn was interrupted before it finished";
  }
  return null;
}
