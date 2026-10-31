import {
  COMPOSER_STACK_DISCLOSURE_TIMEOUT,
  composerStackDisclosureSx,
  swallowStrayDisclosureClick,
} from "./composerStackDisclosure";
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
import {
  CheckCircle,
  ChevronRight,
  Close,
  ExpandLess,
  ExpandMore,
  RadioButtonUnchecked,
} from "@mui/icons-material";
import { alpha } from "@mui/material/styles";
import type { PlanEntry } from "./protocol";
import { memo, type ReactNode } from "react";
import { useStore } from "@cowboy/state-store";
import { useReliableTouchTap } from "./useReliableTouchTap";
import { desktopSize } from "./surface/desktopSize";
import {
  mobileComposerPanelFrameSx,
  mobileComposerPanelHeaderMinHeight,
} from "./mobileComposerPrimitives";
import {
  composerStackExpandedStore,
  toggleComposerStackPanel,
} from "./composerStackAccordion";

// A collapsible, always-visible summary of the agent's current plan (ACP `plan`
// update), docked above the message queue so the task's progress stays in view
// without scrolling the transcript — the Zed-style pinned plan. Collapsed: the
// in-progress step + an N/M counter + a determinate bar. Expanded: the full
// checklist. The plan no longer renders inline in the transcript (it updates in
// place / latest-wins anyway), so this is the single place it lives.

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
  const expanded = useStore(composerStackExpandedStore()) === "plan";
  const total = entries.length;
  const done = entries.filter((e) => e.status === "completed").length;
  const allDone = total > 0 && done === total;
  const pct = total > 0 ? (done / total) * 100 : 0;
  // The step the agent is on — shown inline when collapsed so the bar has
  // context (falls back to the first not-done entry if none is flagged active).
  const active = entries.find((e) => e.status === "in_progress");
  const current = active ?? entries.find((e) => e.status !== "completed");

  const toggle = (): void => {
    toggleComposerStackPanel("plan");
  };
  const toggleTap = useReliableTouchTap<HTMLButtonElement>(
    toggle,
    swallowStrayDisclosureClick,
  );

  if (desktop) {
    return (
      <DesktopPlanRail
        entries={entries}
        expanded={expanded}
        done={done}
        total={total}
        allDone={allDone}
        pct={pct}
        current={current}
        onToggle={toggle}
        onDismiss={onDismiss}
        shortcut={shortcut}
      />
    );
  }

  return (
    <Box
      sx={{
        ...mobileComposerPanelFrameSx,
        bgcolor: "background.default",
        // The progress track owns the panel's bottom edge. Clip it to the same
        // rounded silhouette instead of letting its square layer read as a
        // shelf below the card.
        overflow: "hidden",
      }}
    >
      {/* Header — a standard ripple ButtonBase (the toggle) beside a separate X
          (dismiss). They're siblings, not nested, so the DOM stays valid (no
          button-in-button). Min tap height holds even when the font scale shrinks
          the text (usability over a font-relative row height). */}
      <Stack direction="row" alignItems="center">
        <ButtonBase
          {...toggleTap}
          aria-label={expanded ? "Collapse plan" : "Expand plan"}
          sx={{
            flex: 1,
            minWidth: 0,
            justifyContent: "flex-start",
            textAlign: "left",
            px: 1,
            py: 0.5,
            minHeight: mobileComposerPanelHeaderMinHeight,
            touchAction: "manipulation",
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
      <Collapse
        in={expanded}
        timeout={COMPOSER_STACK_DISCLOSURE_TIMEOUT}
        sx={composerStackDisclosureSx(expanded)}
      >
        {/* Bounded, self-contained scroller — same strategy as the queue/draft
            PendingPanels. Without a cap a long plan grew unbounded and, sitting
            above the composer + the queue/draft scroller, pushed the editor off
            the phone AND chained its touch-scroll into the page (the reported
            jank). `overscrollBehavior: contain` stops the scroll-chaining;
            `maxHeight` keeps the dock from crowding out the editor. */}
        <Stack
          spacing={0.5}
          sx={{
            p: 1.25,
            maxHeight: "30vh",
            // Collapse keeps the checklist mounted. A collapsed
            // `-webkit-overflow-scrolling: touch` tile freezes transcript
            // scroll on iOS, so only the open body is a native scroller.
            overflowY: expanded ? "auto" : "hidden",
            overscrollBehavior: expanded ? "contain" : "auto",
            WebkitOverflowScrolling: expanded ? "touch" : "auto",
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
              >
                <Box
                  sx={{
                    width: "1.25rem",
                    height: "1.25rem",
                    flex: "0 0 1.25rem",
                    display: "grid",
                    placeItems: "center",
                  }}
                >
                  {completed ? (
                    <CheckCircle color="success" sx={{ fontSize: "1.125rem" }} />
                  ) : inProgress ? (
                    <CircularProgress
                      size="1.125rem"
                      thickness={4.5}
                      color="warning"
                    />
                  ) : (
                    <RadioButtonUnchecked color="disabled" sx={{ fontSize: "1.125rem" }} />
                  )}
                </Box>
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

/** Desktop Plan: the same borderless disclosure rail as Calls, Queue and
 * Drafts (chevron column, caption label, quiet summary, N/M), with progress as
 * a hairline along the rail's lower edge instead of the Mobile card's 4px
 * bar. Geometry is root-relative so the rail follows the global font size. */
function DesktopPlanRail({
  entries,
  expanded,
  done,
  total,
  allDone,
  pct,
  current,
  onToggle,
  onDismiss,
  shortcut,
}: {
  entries: PlanEntry[];
  expanded: boolean;
  done: number;
  total: number;
  allDone: boolean;
  pct: number;
  current: PlanEntry | undefined;
  onToggle: () => void;
  onDismiss: () => void;
  shortcut?: ReactNode;
}): React.JSX.Element {
  return (
    <Box
      data-desktop-plan-surface="true"
      sx={{
        position: "relative",
        border: 0,
        borderBottom: 1,
        borderColor: "divider",
        borderRadius: 0,
        bgcolor: "transparent",
        mb: 0.5,
        color: "text.primary",
        overflow: "hidden",
        transition: "background-color 120ms ease, border-color 120ms ease",
        "&:focus-within": { borderColor: "primary.main" },
      }}
    >
      <Stack direction="row" alignItems="center" sx={{ pr: 0.5 }}>
        <ButtonBase
          onClick={onToggle}
          disableRipple
          aria-label={expanded ? "Collapse plan" : "Expand plan"}
          aria-expanded={expanded}
          sx={{
            flex: 1,
            minWidth: 0,
            alignSelf: "stretch",
            justifyContent: "flex-start",
            textAlign: "left",
            pl: 0.5,
            pr: 0.5,
            minHeight: desktopSize(32),
            "&:active, &:focus-visible": { bgcolor: "action.hover" },
          }}
        >
          <Stack
            direction="row"
            alignItems="center"
            // rem gap + 22 chevron column = Queue/Drafts' 28 column, at any scale.
            sx={{ width: "100%", minWidth: 0, gap: "0.375rem" }}
          >
            <Box
              sx={{
                width: desktopSize(22),
                display: "inline-flex",
                justifyContent: "center",
                flexShrink: 0,
              }}
            >
              {expanded
                ? <ExpandMore fontSize="small" />
                : <ChevronRight fontSize="small" />}
            </Box>
            <Typography variant="caption" sx={{ fontWeight: 600, flexShrink: 0 }}>
              Plan
            </Typography>
            <Typography
              variant="caption"
              noWrap
              sx={{
                flex: 1,
                minWidth: 0,
                color: allDone ? "success.main" : "text.secondary",
              }}
            >
              {allDone ? "All steps complete" : (current?.content ?? "")}
            </Typography>
            {shortcut}
            <Typography
              variant="caption"
              color={allDone ? "success.main" : "text.secondary"}
              sx={{ fontWeight: 600, fontVariantNumeric: "tabular-nums", flexShrink: 0 }}
            >
              {done}/{total}
            </Typography>
          </Stack>
        </ButtonBase>
        <Tooltip title="Dismiss plan">
          <IconButton
            aria-label="Dismiss plan"
            onClick={onDismiss}
            sx={{
              width: desktopSize(32),
              height: desktopSize(32),
              p: 0,
              flexShrink: 0,
              color: "text.secondary",
              "& .MuiSvgIcon-root": {
                fontSize: "1rem",
                width: "1rem",
                height: "1rem",
              },
            }}
          >
            <Close />
          </IconButton>
        </Tooltip>
      </Stack>
      <Box
        aria-hidden
        sx={{
          position: "absolute",
          left: 0,
          bottom: 0,
          height: 2,
          width: `${String(pct)}%`,
          bgcolor: allDone ? "success.main" : "primary.main",
          opacity: allDone ? 0.7 : 0.55,
          transition: "width 240ms ease",
        }}
      />
      <Collapse
        in={expanded}
        timeout={COMPOSER_STACK_DISCLOSURE_TIMEOUT}
        sx={composerStackDisclosureSx(expanded)}
      >
        <Box
          data-desktop-plan-list="true"
          data-desktop-aux-list="true"
          sx={{
            maxHeight: 176,
            overflowY: expanded ? "auto" : "hidden",
            overscrollBehavior: "contain",
            pb: 0.5,
            transition: "max-height 150ms ease, padding 150ms ease",
          }}
        >
          {entries.map((entry, index) => {
            const completed = entry.status === "completed";
            const inProgress = entry.status === "in_progress";
            return (
              <Stack
                key={index}
                direction="row"
                alignItems="center"
                data-desktop-item={`plan-${String(index)}`}
                tabIndex={-1}
                sx={{
                  pl: 0.5,
                  pr: 1,
                  gap: "0.375rem",
                  minHeight: desktopSize(28),
                  outline: "none",
                  transition: "background-color 120ms ease, box-shadow 120ms ease",
                  "&:hover": {
                    bgcolor: (t) => alpha(t.palette.primary.main, 0.025),
                  },
                  "&:focus": {
                    bgcolor: (t) => alpha(t.palette.primary.main, 0.05),
                    boxShadow: (t) => `inset 2px 0 0 ${t.palette.primary.main}`,
                  },
                }}
              >
                <Box
                  sx={{
                    width: desktopSize(22),
                    display: "inline-flex",
                    justifyContent: "center",
                    flexShrink: 0,
                    "& .MuiSvgIcon-root": {
                      fontSize: "0.875rem",
                      width: "0.875rem",
                      height: "0.875rem",
                    },
                  }}
                >
                  {completed
                    ? <CheckCircle color="success" />
                    : inProgress
                    ? <CircularProgress size="0.75rem" thickness={5} color="warning" />
                    : <RadioButtonUnchecked color="disabled" />}
                </Box>
                <Typography
                  variant="caption"
                  color={completed ? "text.disabled" : "text.primary"}
                  sx={{
                    minWidth: 0,
                    fontWeight: inProgress ? 600 : 400,
                    textDecoration: completed ? "line-through" : "none",
                  }}
                >
                  {entry.content}
                </Typography>
              </Stack>
            );
          })}
        </Box>
      </Collapse>
    </Box>
  );
}
