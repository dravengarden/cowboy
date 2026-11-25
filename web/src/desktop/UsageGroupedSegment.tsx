import type { ReactNode } from "react";
import { Box, Stack, Typography } from "@mui/material";
import type { UsageAccountCard } from "../usageAccounts";
import type { UsageWidgetGroup } from "../usageWidget";
import { usageCountdown, usageRemainingTone, type UsageTone } from "./topBarDensity";

/** One window cell of a quota group row: `5h 34%`. */
export type UsageGroupCell = {
  period: string;
  primary: string;
  remaining: number;
  /** Time to reset, beside every window that has spent quota: "when does
   *  this come back?" is the question a quota cell exists to answer, and the
   *  segment budget already reserves the room. A full window has nothing to
   *  recover, so it omits it; the hover still carries every countdown. */
  countdown?: string;
  /** The window is low enough that its countdown takes the warning colour. */
  attention: boolean;
};

/** One quota group as a row of the grouped segment. */
export type UsageGroupRow = {
  label: string;
  cells: UsageGroupCell[];
  /** Every window of the group with its countdown, for the hover. */
  detail: string;
};

/** `Weekly` is the widest label and the most repeated one; a column of `W`
 *  next to a column of `5h` stays unambiguous. */
function shortPeriodLabel(label: string): string {
  return label === "Weekly" ? "W" : label;
}

/** Per-group quotas (Antigravity) as a matrix: each group a row, each window
 *  a labelled cell, so `Gemini  5h 34%  W 28%` reads left to right without
 *  matching a number to a caption on the line below. */
export function usageGroupRows(groups: readonly UsageWidgetGroup[], now: number): UsageGroupRow[] {
  return groups.map((group) => ({
    label: group.label,
    cells: group.windows.map((window): UsageGroupCell => {
      const countdown = usageCountdown(window.resetsAt, now);
      return {
        period: shortPeriodLabel(window.periodLabel),
        primary: `${String(window.remaining)}%`,
        remaining: window.remaining,
        attention: usageRemainingTone(window.remaining) !== "normal",
        ...(countdown !== undefined && window.remaining < 100 ? { countdown } : {}),
      };
    }),
    detail: `${group.label}: ${group.windows
      .map((window) => {
        const countdown = usageCountdown(window.resetsAt, now);
        return `${window.periodLabel} ${String(window.remaining)}%${
          countdown === undefined ? "" : ` (${countdown})`
        }`;
      })
      .join(" · ")}`,
  }));
}

export const USAGE_TONE_COLOR: Record<UsageTone, string> = {
  critical: "error.main",
  low: "warning.main",
  normal: "text.primary",
};

/** A top-bar segment of per-group quotas (Antigravity). */
export function UsageGroupedSegment({
  groups,
  kind,
  account,
  refreshing,
  now,
  nameCell,
  label,
  sx,
}: {
  groups: readonly UsageWidgetGroup[];
  kind: string;
  account: UsageAccountCard;
  refreshing: boolean;
  now: number;
  /** The account name and its refresh spinner. */
  nameCell: ReactNode;
  label: string;
  /** The segment surface shared with every other account. */
  sx: Record<string, unknown>;
}): React.JSX.Element {
  // Per-group quotas read as a small matrix: the account name spans the
  // rows, each group is a row, each window a labelled cell. The window
  // label sits beside its number, so nothing has to be matched against a
  // caption below it, and the windows of every group align as columns.
  const rows = usageGroupRows(groups, now);
  const windowColumns = Math.max(1, ...rows.map((row) => row.cells.length));
  // Two rows fill the strip's height; a third shown group tightens them.
  const rowSx = {
    fontSize: "0.6875rem",
    lineHeight: rows.length > 2 ? 1.15 : 1.5,
  } as const;
  return (
    <Box
      data-usage-provider={kind}
      data-usage-account={account.id}
      data-usage-grouped=""
      title={`${account.title}${refreshing ? " · Refreshing…" : ""}\n${rows
        .map((row) => row.detail)
        .join("\n")}`}
      aria-label={label}
      sx={{
        ...sx,
        alignItems: "center",
        gridTemplateColumns: `auto auto repeat(${String(windowColumns)}, auto)`,
        columnGap: 1,
      }}
    >
      <Stack
        direction="row"
        spacing={0.55}
        alignItems="center"
        sx={{ gridRow: `1 / span ${String(rows.length)}`, minWidth: 0, pr: 0.25 }}
      >
        {nameCell}
      </Stack>
      {rows.map((row, rowIndex) => (
        <Box key={row.label} sx={{ display: "contents" }}>
          <Typography
            variant="caption"
            color="text.secondary"
            fontWeight={500}
            noWrap
            sx={{ ...rowSx, gridRow: rowIndex + 1, minWidth: 0 }}
          >
            {row.label}
          </Typography>
          {row.cells.map((cell, cellIndex) => (
            <Stack
              key={`${row.label}-${String(cellIndex)}`}
              direction="row"
              spacing={0.5}
              alignItems="baseline"
              sx={{ gridRow: rowIndex + 1, gridColumn: cellIndex + 3, minWidth: 0 }}
            >
              <Typography
                variant="caption"
                color="text.secondary"
                noWrap
                sx={{ ...rowSx, fontSize: "0.625rem" }}
              >
                {cell.period}
              </Typography>
              <Typography
                variant="caption"
                fontWeight={800}
                noWrap
                sx={{
                  ...rowSx,
                  flexShrink: 0,
                  // Right-aligned in a fixed-width figure so `34%` and
                  // `100%` share a right edge down the column.
                  minWidth: "4ch",
                  textAlign: "right",
                  fontVariantNumeric: "tabular-nums",
                  color: USAGE_TONE_COLOR[usageRemainingTone(cell.remaining)],
                }}
              >
                {cell.primary}
              </Typography>
              {cell.countdown !== undefined && (
                <Typography
                  variant="caption"
                  noWrap
                  sx={{
                    ...rowSx,
                    fontSize: "0.625rem",
                    fontVariantNumeric: "tabular-nums",
                    // Quiet by default so the percentages stay the first read;
                    // a window that needs attention lifts its countdown to the
                    // percentage's tone.
                    color: cell.attention
                      ? USAGE_TONE_COLOR[usageRemainingTone(cell.remaining)]
                      : "text.secondary",
                  }}
                >
                  {cell.countdown}
                </Typography>
              )}
            </Stack>
          ))}
        </Box>
      ))}
    </Box>
  );
}
