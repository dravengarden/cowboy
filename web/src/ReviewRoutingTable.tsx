import React from "react";
import { Box, Typography } from "@mui/material";
import type { ReviewRouting, RoutingCost, RoutingRow } from "./agentTools";

const FAMILY: Record<string, string> = { astra: "Astra", sol: "SOL", opus: "Opus", fable: "Fable" };
const PROVIDER: Record<string, string> = { codex: "GPT", "claude-code": "Claude" };

/** `model · effort` for a row, e.g. `gpt-6.2-sol · max` or `Opus · high`. */
export function rowLabel(row: RoutingRow): string {
  const model = row.model ?? FAMILY[row.family] ?? row.family;
  return row.efforts?.length ? `${model} · ${row.efforts.join("/")}` : model;
}

function tokens(value: number): string {
  return value >= 1_000_000 ? `${(value / 1_000_000).toFixed(1)}M` : `${Math.round(value / 1000)}k`;
}

/** Measured cost of a row: field median, then the paired pilot's recall. */
export function costLabel(row: RoutingRow, costs: Record<string, RoutingCost>): string {
  const model = row.model ?? row.family;
  const cost = (row.efforts ?? []).map((effort) => costs[`${model}:${effort}`]).find(Boolean);
  if (!cost) return "cost unknown";
  const parts: string[] = [];
  if (cost.field) parts.push(`${tokens(cost.field.median_tokens)} tokens, ${cost.field.median_minutes} min (n=${cost.field.n})`);
  if (cost.serious_recall) parts.push(`pilot ${cost.serious_recall}`);
  return parts.length ? parts.join(" · ") : "cost unknown";
}

/** The effective routing table, read-only: who reviews each tier and what it costs. */
export function ReviewRoutingTable({ routing }: { routing: ReviewRouting }): React.JSX.Element {
  const tiers = [["High risk", "high"], ["Routine", "routine"], ["Economy routine", "economy"]] as const;
  return (
    <Box data-review-routing sx={{ py: 0.75 }}>
      <Typography variant="body2">Review routing</Typography>
      <Typography variant="caption" color="text.secondary" component="p">
        The table's first choice per tier, before your family lists; a session never reviews with its own model.
        Evidence {routing.evidence}; tokens are code-review field medians, not quota.
      </Typography>
      {Object.entries(routing.providers).map(([provider, table]) => (
        <Box key={provider} sx={{ mt: 0.75 }}>
          <Typography variant="caption" sx={{ fontWeight: 600 }}>{PROVIDER[provider] ?? provider}</Typography>
          {tiers.map(([label, key]) => {
            const row = table[key]?.auto[0];
            if (!row) return null;
            return (
              <Box key={key} sx={{ display: "flex", gap: 1, flexWrap: "wrap" }}>
                <Typography variant="caption" color="text.secondary" sx={{ minWidth: "7.5rem" }}>{label}</Typography>
                <Typography variant="caption">{rowLabel(row)}</Typography>
                <Typography variant="caption" color="text.secondary">{costLabel(row, routing.costs)}</Typography>
              </Box>
            );
          })}
        </Box>
      ))}
    </Box>
  );
}
