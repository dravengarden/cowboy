// Zero-extra-row Mobile Background Tasks entry pinned at the fixed trailing edge
// of the Composer action row (outside the horizontally scrolling tool track).
// Stays visible when tasks exist (including quiet completed history) and when
// the software keyboard is open; vanishes completely (0px) when 0 tasks exist.

import { memo } from "react";
import { alpha, Box, IconButton, Tooltip } from "@mui/material";
import { AccountTreeOutlined } from "@mui/icons-material";
import { openBackgroundTask } from "../backgroundTaskInspectorState";
import { useBackgroundTasks } from "../useBackgroundTasks";
import { taskTabSummary } from "./taskTree";

export const MobileTasksButton = memo(function MobileTasksButton({
  sessionId,
  onBeforeOpen,
}: {
  sessionId: string;
  /** Let the Composer persist draft/caret & settle/dismiss the soft keyboard. */
  onBeforeOpen?: () => void;
}): React.JSX.Element | null {
  const { tasks, backgroundActivity } = useBackgroundTasks(sessionId);
  const summary = taskTabSummary(tasks, backgroundActivity);
  if (!summary.hasTasks) return null;

  const badgeTone = summary.needsYouCount > 0
    ? "warning"
    : summary.failedUnseenCount > 0
    ? "error"
    : summary.activeCount > 0 || summary.extraActivity
    ? "info"
    : null;
  const badgeText = summary.needsYouCount > 0
    ? String(summary.needsYouCount)
    : summary.failedUnseenCount > 0
    ? "!"
    : summary.activeCount > 0
    ? String(summary.activeCount)
    : summary.extraActivity
    ? "•"
    : null;

  const statusWords: string[] = [];
  if (summary.needsYouCount > 0) statusWords.push(`${String(summary.needsYouCount)} needs you`);
  if (summary.failedUnseenCount > 0) statusWords.push(`${String(summary.failedUnseenCount)} failed`);
  if (summary.activeCount > 0) statusWords.push(`${String(summary.activeCount)} active`);
  if (summary.quietHistory) statusWords.push(`${String(summary.totalCount)} ended`);
  const ariaLabel = `Background tasks${statusWords.length > 0 ? `: ${statusWords.join(", ")}` : ""}`;

  return (
    <Box
      data-mobile-tasks-slot={sessionId}
      sx={(theme) => ({
        flexShrink: 0,
        display: "inline-flex",
        alignItems: "center",
        pl: 0.25,
        pr: 0.5,
        borderLeft: 1,
        borderColor: alpha(theme.palette.divider, 0.7),
      })}
    >
      <Tooltip title={ariaLabel}>
        <IconButton
          data-mobile-tasks-entry={sessionId}
          data-mobile-tasks-tone={badgeTone ?? "quiet"}
          aria-label={ariaLabel}
          aria-haspopup="dialog"
          onClick={(event): void => {
            onBeforeOpen?.();
            // Always open Triage Peek (key=null) so 1 tap shows all statuses
            // above the thumb without forcing a Back detour.
            openBackgroundTask(sessionId, null, "bar", event.currentTarget);
          }}
          sx={{
            position: "relative",
            width: 44,
            height: 44,
            flexShrink: 0,
            color: badgeTone === "warning"
              ? "warning.main"
              : badgeTone === "error"
              ? "error.main"
              : badgeTone === "info"
              ? "primary.main"
              : "text.secondary",
          }}
        >
          <AccountTreeOutlined fontSize="small" />
          {badgeTone && badgeText && (
            <Box
              component="span"
              data-mobile-tasks-badge={badgeTone}
              sx={(theme) => ({
                position: "absolute",
                top: 5,
                right: 5,
                minWidth: 16,
                height: 16,
                px: 0.4,
                borderRadius: 999,
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                fontSize: "0.625rem",
                fontWeight: 800,
                lineHeight: 1,
                color: theme.palette.getContrastText(theme.palette[badgeTone].main),
                bgcolor: theme.palette[badgeTone].main,
                boxShadow: `0 0 0 1.5px ${theme.palette.background.paper}`,
              })}
            >
              {badgeText}
            </Box>
          )}
        </IconButton>
      </Tooltip>
    </Box>
  );
});
