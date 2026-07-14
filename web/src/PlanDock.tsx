import {
  Box,
  ButtonBase,
  Collapse,
  CircularProgress,
  IconButton,
  LinearProgress,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import { CheckCircle, Close, ExpandLess, ExpandMore, RadioButtonUnchecked } from "@mui/icons-material";
import type { PlanEntry } from "./protocol";
import { memo, type ReactNode } from "react";
import { persisted, useStore } from "./_store/mod.ts";

// A collapsible, always-visible summary of the agent's current plan (ACP `plan`
// update), docked above the message queue so the task's progress stays in view
// without scrolling the transcript — the Zed-style pinned plan. Collapsed: the
// in-progress step + an N/M counter + a determinate bar. Expanded: the full
// checklist. The plan no longer renders inline in the transcript (it updates in
// place / latest-wins anyway), so this is the single place it lives.

// Persist the collapse choice globally — the dock remounts per session (with the
// composer), so a local-only state would forget the user's preference on switch.
// "1"/"0" legacy format preserved.
const planExpanded = persisted("cowboy:plan-expanded", false, {
  serialize: (v) => (v ? "1" : "0"),
  deserialize: (s) => s === "1",
});

export const PlanDock = memo(function PlanDock({
  entries,
  onDismiss,
  desktop = false,
  shortcut,
}: {
  entries: PlanEntry[];
  /** Manually close the dock (the X). The plan stays hidden until a new/different
   *  plan arrives — see Composer's dismissedPlanKey. */
  onDismiss: () => void;
  /** Desktop owns focus-driven workspace sizing; Mobile keeps the bounded
   * touch scroller that prevents the composer from leaving the viewport. */
  desktop?: boolean;
  /** Desktop-only persistent global navigation hint. */
  shortcut?: ReactNode;
}): React.JSX.Element {
  const expanded = useStore(planExpanded);
  const total = entries.length;
  const done = entries.filter((e) => e.status === "completed").length;
  const allDone = total > 0 && done === total;
  const pct = total > 0 ? (done / total) * 100 : 0;
  // The step the agent is on — shown inline when collapsed so the bar has
  // context (falls back to the first not-done entry if none is flagged active).
  const active = entries.find((e) => e.status === "in_progress");
  const current = active ?? entries.find((e) => e.status !== "completed");

  const toggle = (): void => {
    planExpanded.set(!expanded);
  };

  return (
    <Box
      data-desktop-plan-surface={desktop ? "true" : undefined}
      sx={{
        border: 1,
        borderColor: "divider",
        borderRadius: 1.5,
        mb: 1,
        bgcolor: "background.default",
        overflow: "hidden",
        ...(desktop && {
          transition: "background-color 120ms ease, border-color 120ms ease",
        }),
      }}
    >
      {/* Header — a standard ripple ButtonBase (the toggle) beside a separate X
          (dismiss). They're siblings, not nested, so the DOM stays valid (no
          button-in-button). Min tap height holds even when the font scale shrinks
          the text (usability over a font-relative row height). */}
      <Stack direction="row" alignItems="center">
        <ButtonBase
          onClick={toggle}
          aria-label={expanded ? "Collapse plan" : "Expand plan"}
          sx={{
            flex: 1,
            minWidth: 0,
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
            {shortcut}
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
        <Tooltip title="Dismiss plan">
          <IconButton
            size="small"
            aria-label="Dismiss plan"
            onClick={onDismiss}
            sx={{ mx: 0.25, flexShrink: 0, color: "text.secondary" }}
          >
            <Close fontSize="small" />
          </IconButton>
        </Tooltip>
      </Stack>
      {/* Always visible (even collapsed) so progress reads at a glance. */}
      <LinearProgress
        variant="determinate"
        value={pct}
        color={allDone ? "success" : "primary"}
        sx={{ height: 4 }}
      />
      <Collapse in={expanded}>
        {/* Bounded, self-contained scroller — same strategy as the queue/draft
            PendingPanels. Without a cap a long plan grew unbounded and, sitting
            above the composer + the queue/draft scroller, pushed the editor off
            the phone AND chained its touch-scroll into the page (the reported
            jank). `overscrollBehavior: contain` stops the scroll-chaining;
            `maxHeight` keeps the dock from crowding out the editor. */}
        <Stack
          spacing={0.5}
          data-desktop-plan-list={desktop ? "true" : undefined}
          data-desktop-aux-list={desktop ? "true" : undefined}
          sx={{
            p: 1.25,
            maxHeight: desktop ? 176 : "30vh",
            overflowY: "auto",
            overscrollBehavior: "contain",
            WebkitOverflowScrolling: "touch",
            ...(desktop && {
              transition: "max-height 150ms ease, padding 150ms ease",
              "[data-desktop-focused='true'] &": {
                maxHeight: "min(46vh, 560px)",
              },
            }),
          }}
        >
          {entries.map((e, j) => {
            const completed = e.status === "completed";
            const inProgress = e.status === "in_progress";
            return (
              <Stack
                key={j}
                direction="row"
                spacing={1}
                alignItems="center"
                {...(desktop
                  ? {
                    "data-desktop-item": `plan-${String(j)}`,
                    tabIndex: -1,
                  }
                  : {})}
              >
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
});
