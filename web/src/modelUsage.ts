// Per-model token usage of one session. The worker adds up each prompt
// response's `_meta.quota.model_usage` (claude-agent-acp and codex-acp) and
// carries the totals on the session usage as `raw._cowboy.modelUsage`; see
// src/acp_model_usage.rs. Claude's rows also count its Task subagents.

import type { SessionMeta } from "./protocol";

export interface ModelUsageRow {
  model: string;
  totalTokens: number;
  inputTokens: number;
  outputTokens: number;
  turns: number;
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function tokens(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

/** The session's models, most tokens first. Empty when none was reported. */
export function sessionModelUsage(session: Pick<SessionMeta, "usage">): ModelUsageRow[] {
  const rows = record(record(session.usage?.raw)?.["_cowboy"])?.["modelUsage"];
  if (!Array.isArray(rows)) return [];
  return rows.flatMap((value): ModelUsageRow[] => {
    const row = record(value);
    const model = typeof row?.["model"] === "string" ? row["model"].trim() : "";
    const total = tokens(row?.["totalTokens"]);
    if (!row || !model || total === 0) return [];
    return [{
      model,
      totalTokens: total,
      inputTokens: tokens(row["inputTokens"]),
      outputTokens: tokens(row["outputTokens"]),
      turns: tokens(row["turns"]),
    }];
  }).sort((left, right) => right.totalTokens - left.totalTokens);
}

/** The model the latest context report belongs to (claude-agent-acp
 *  `_meta["_claude/model"]` on `usage_update`), if the agent names it. */
export function contextModel(session: Pick<SessionMeta, "usage">): string | null {
  const model = record(record(session.usage?.raw)?.["_meta"])?.["_claude/model"];
  return typeof model === "string" && model.trim() ? model.trim() : null;
}

/** 950, 12.3k, 4.1M. */
export function compactTokens(value: number): string {
  if (value < 1000) return String(value);
  const scaled = (divisor: number): string => {
    const amount = value / divisor;
    return amount < 10 ? amount.toFixed(1).replace(/\.0$/, "") : String(Math.round(amount));
  };
  const thousands = scaled(1000);
  return Number(thousands) < 1000 ? `${thousands}k` : `${scaled(1_000_000)}M`;
}

/** "1.2M tokens · 3 turns". */
export function modelUsageSummary(row: ModelUsageRow): string {
  const parts = [`${compactTokens(row.totalTokens)} tokens`];
  if (row.turns > 0) parts.push(`${String(row.turns)} ${row.turns === 1 ? "turn" : "turns"}`);
  return parts.join(" · ");
}
