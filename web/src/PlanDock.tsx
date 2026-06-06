import { useState } from "react";
import {
  Box,
  ButtonBase,
  Collapse,
  CircularProgress,
  LinearProgress,
  Stack,
  Typography,
} from "@mui/material";
import { CheckCircle, ExpandLess, ExpandMore, RadioButtonUnchecked } from "@mui/icons-material";
import type { PlanEntry } from "./protocol";

// A collapsible, always-visible summary of the agent's current plan (ACP `plan`
// update), docked above the message queue so the task's progress stays in view
// without scrolling the transcript — the Zed-style pinned plan. Collapsed: the
// in-progress step + an N/M counter + a determinate bar. Expanded: the full
// checklist. The plan no longer renders inline in the transcript (it updates in
// place / latest-wins anyway), so this is the single place it lives.

// Persist the collapse choice globally — the dock remounts per session (with the
// composer), so a local-only state would forget the user's preference on switch.
const KEY = "cowboy:plan-expanded";
function readExpanded(): boolean {
  return globalThis.localStorage?.getItem(KEY) === "1";
}

export function PlanDock({ entries }: { entries: PlanEntry[] }): React.JSX.Element {
  const [expanded, setExpanded] = useState(readExpanded);
  const total = entries.length;
  const done = entries.filter((e) => e.status === "completed").length;
  const allDone = total > 0 && done === total;
  const pct = total > 0 ? (done / total) * 100 : 0;
  // The step the agent is on — shown inline when collapsed so the bar has
  // context (falls back to the first not-done entry if none is flagged active).
  const active = entries.find((e) => e.status === "in_progress");
  const current = active ?? entries.find((e) => e.status !== "completed");

  const toggle = (): void => {
    setExpanded((prev) => {
      const next = !prev;
      globalThis.localStorage?.setItem(KEY, next ? "1" : "0");
      return next;
    });
  };

  return (
    <Box
      sx={{
        border: 1,
        borderColor: "divider",
        borderRadius: 1.5,
        mb: 1,
        bgcolor: "background.default",
        overflow: "hidden",
      }}
    >
      {/* Header — a standard ripple ButtonBase so the whole row is one tap target
          with Material feedback, and a min tap height that holds even when the
          font scale shrinks the text (usability over a font-relative row height). */}
      <ButtonBase
        onClick={toggle}
        aria-label={expanded ? "Collapse plan" : "Expand plan"}
        sx={{
          width: "100%",
          justifyContent: "flex-start",
          textAlign: "left",
          px: 1,
          py: 0.5,
          minHeight: 40,
          "@media (pointer: coarse)": { minHeight: 44 },
        }}
      >
        <Stack direction="row" alignItems="center" spacing={1} sx={{ width: "100%", minWidth: 0 }}>
          {expanded ? (
            <ExpandLess fontSize="small" sx={{ color: "text.secondary" }} />
          ) : (
            <ExpandMore fontSize="small" sx={{ color: "text.secondary" }} />
          )}
          <Typography variant="overline" sx={{ lineHeight: 1.4 }}>
            Plan
          </Typography>
          {!expanded && (
            <Typography
              variant="body2"
              noWrap
              sx={{ flex: 1, minWidth: 0, color: allDone ? "success.main" : "text.secondary" }}
            >
              {allDone ? "All steps complete" : (current?.content ?? "")}
            </Typography>
          )}
          {expanded && <Box sx={{ flex: 1 }} />}
          {allDone && <CheckCircle fontSize="small" color="success" />}
          <Typography
            variant="caption"
            color={allDone ? "success.main" : "text.secondary"}
            sx={{ fontWeight: 600, fontVariantNumeric: "tabular-nums" }}
          >
            {done}/{total}
          </Typography>
        </Stack>
      </ButtonBase>
      {/* Always visible (even collapsed) so progress reads at a glance. */}
      <LinearProgress
        variant="determinate"
        value={pct}
        color={allDone ? "success" : "primary"}
        sx={{ height: 4 }}
      />
      <Collapse in={expanded}>
        <Stack spacing={0.5} sx={{ p: 1.25 }}>
          {entries.map((e, j) => {
            const completed = e.status === "completed";
            const inProgress = e.status === "in_progress";
            return (
              <Stack key={j} direction="row" spacing={1} alignItems="center">
                {completed ? (
                  <CheckCircle fontSize="small" color="success" />
                ) : inProgress ? (
                  <CircularProgress size={16} thickness={5} color="warning" sx={{ mx: 0.25 }} />
                ) : (
                  <RadioButtonUnchecked fontSize="small" color="disabled" />
                )}
                <Typography
                  variant="body2"
                  color={completed ? "text.disabled" : "text.primary"}
                  sx={{
                    fontWeight: inProgress ? 600 : 400,
                    textDecoration: completed ? "line-through" : "none",
                  }}
                >
                  {e.content}
                </Typography>
              </Stack>
            );
          })}
        </Stack>
      </Collapse>
    </Box>
  );
}
