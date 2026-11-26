import {
  COMPACTING_NOTICE,
  type ContentChunk,
  isCompactionCommandText,
  isCompactionCompletionText,
  isHumanPrompt,
  type RenderItem,
} from "./derive";

export interface TurnForkPoint {
  /** 1-based ordinal of this completed turn within the current context epoch. */
  turnNumber: number;
  /** Stable RenderItem.key of the turn's final assistant message. */
  itemKey: string;
  /** Provider message ID when the agent emitted one (enables Native Fork). */
  messageId?: string | undefined;
  /** Exclusive sequence cutoff (`lastSeq + 1`) for Linked Fork fallback. */
  beforeSeq: number;
  /** Compact single-line summary of the user prompt that opened this turn. */
  promptPreview: string;
  /** Compact single-line summary of the turn's final assistant response. */
  answerPreview: string;
}

function chunksText(chunks: readonly ContentChunk[]): string {
  return chunks
    .filter((chunk): chunk is Extract<ContentChunk, { type: "text" }> => chunk.type === "text")
    .map((chunk) => chunk.text)
    .join("")
    .trim();
}

function isCompactionAssistantChunks(chunks: readonly ContentChunk[]): boolean {
  if (chunks.length === 0 || chunks.some((chunk) => chunk.type !== "text")) return false;
  const text = chunksText(chunks);
  return text === COMPACTING_NOTICE || isCompactionCompletionText(text);
}

function compactPreview(chunks: readonly ContentChunk[], maxLen: number): string {
  const cleaned = chunksText(chunks)
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^[#>*+\-\d.\s`]+/, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return "";
  return cleaned.length > maxLen ? `${cleaned.slice(0, maxLen - 1).trimEnd()}…` : cleaned;
}

/** Extract every completed turn-final assistant message in the current context
 *  epoch, strictly mirroring `src/core/session_fork.rs::Hub::fork_point`:
 *  - Only the LAST non-compaction assistant message of a completed turn qualifies.
 *  - Steered user messages stay inside the running turn and do not settle it.
 *  - Any `cleared` boundary discards earlier turns (native and linked forks
 *    cannot reference messages behind a context reset).
 *  - While `sourceBusy` is true, the trailing turn is still in flight and none
 *    of its intermediate commentary bubbles are offered as fork points. */
export function extractCompletedTurnForkPoints(
  items: readonly RenderItem[],
  sourceBusy: boolean,
): TurnForkPoint[] {
  const turns: TurnForkPoint[] = [];
  let currentPrompt = "";
  let lastAssistant: Extract<RenderItem, { kind: "message" }> | null = null;

  const settle = (): void => {
    if (lastAssistant !== null) {
      const seq = Number(lastAssistant.key);
      const lastSeq = lastAssistant.lastSeq ?? (Number.isFinite(seq) ? seq : undefined);
      if (lastSeq !== undefined && lastSeq > 0) {
        const turnNumber = turns.length + 1;
        turns.push({
          turnNumber,
          itemKey: lastAssistant.key,
          ...(lastAssistant.messageId ? { messageId: lastAssistant.messageId } : {}),
          beforeSeq: lastSeq + 1,
          promptPreview: currentPrompt || `Turn ${String(turnNumber)}`,
          answerPreview: compactPreview(lastAssistant.chunks, 96),
        });
      }
    }
    lastAssistant = null;
    currentPrompt = "";
  };

  for (const item of items) {
    if (item.kind === "cleared") {
      turns.length = 0;
      lastAssistant = null;
      currentPrompt = "";
      continue;
    }
    if (item.kind === "message" && item.role === "user") {
      if (item.steered) continue;
      settle();
      const rawText = chunksText(item.chunks);
      if (isHumanPrompt(item.origin) && !isCompactionCommandText(rawText)) {
        currentPrompt = compactPreview(item.chunks, 68);
      }
      continue;
    }
    if (item.kind === "message" && item.role === "assistant") {
      if (isCompactionAssistantChunks(item.chunks)) continue;
      lastAssistant = item;
    }
  }

  if (!sourceBusy) settle();
  return turns;
}

const mapCache = new WeakMap<
  readonly RenderItem[],
  { idle?: ReadonlyMap<string, TurnForkPoint>; busy?: ReadonlyMap<string, TurnForkPoint> }
>();

/** Cached O(1) lookup map keyed by RenderItem.key. Preserves referential
 *  identity across renders so `memo(ItemView)` does not re-render settled rows
 *  and Transcript requires zero additional React hooks. */
export function completedTurnForkPointMap(
  items: readonly RenderItem[],
  sourceBusy: boolean,
): ReadonlyMap<string, TurnForkPoint> {
  let entry = mapCache.get(items);
  if (!entry) {
    entry = {};
    mapCache.set(items, entry);
  }
  const existing = sourceBusy ? entry.busy : entry.idle;
  if (existing) return existing;
  const built = new Map<string, TurnForkPoint>();
  for (const turn of extractCompletedTurnForkPoints(items, sourceBusy)) {
    built.set(turn.itemKey, turn);
  }
  if (sourceBusy) entry.busy = built;
  else entry.idle = built;
  return built;
}

/** Highest `cleared` sequence in `items`, or `null` when none exists. */
export function latestContextClearSeq(items: readonly RenderItem[]): number | null {
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const item = items[i];
    if (item?.kind === "cleared") {
      const seq = Number(item.key);
      return Number.isFinite(seq) ? seq : 0;
    }
  }
  return null;
}

export function formatTurnForkLabel(
  turn: Pick<TurnForkPoint, "turnNumber" | "promptPreview">,
): string {
  const prefix = `Turn ${String(turn.turnNumber)}`;
  if (!turn.promptPreview || turn.promptPreview === prefix) return prefix;
  return `${prefix} · ${turn.promptPreview}`;
}
