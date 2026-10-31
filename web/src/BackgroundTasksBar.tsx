// The one-line background-tasks bar in the Composer stack (replaces the Calls
// panel; docs/background-tasks.md §6.3), e.g.
//   ◐ 2 working · ⚠ 1 needs you · ✕ 1 failed — Review: auth fallback 1m40s ›
// It counts only TaskSummary items and hides when nothing is active or needs
// attention; the history stays in the inspector (`␣G` opens it regardless).

import { memo, type ReactNode } from "react";
import { Box, ButtonBase, Typography } from "@mui/material";
import { ChevronRight } from "@mui/icons-material";
import { barSummary, CALLS_DOCK_LEGACY_KEY, type TaskSummary } from "./backgroundTasks";
import { openBackgroundTask } from "./backgroundTaskInspectorState";
import { desktopSize } from "./surface/desktopSize";
import { mobileComposerPanelHeaderMinHeight } from "./mobileComposerPrimitives";
import { useBackgroundTasks } from "./useBackgroundTasks";

export { CALLS_DOCK_LEGACY_KEY };

export const BackgroundTasksBar = memo(function BackgroundTasksBar({
  sessionId,
  desktop,
  shortcut,
}: {
  sessionId: string;
  desktop: boolean;
  shortcut?: ReactNode;
}): React.JSX.Element | null {
  const { tasks, backgroundActivity } = useBackgroundTasks(sessionId);
  return (
    <BackgroundTasksBarView
      sessionId={sessionId}
      desktop={desktop}
      shortcut={shortcut}
      tasks={tasks}
      backgroundActivity={backgroundActivity}
    />
  );
});

/** The bar for already summarised tasks (fixtures render it directly). */
export function BackgroundTasksBarView({
  sessionId,
  desktop,
  shortcut,
  tasks,
  backgroundActivity = 0,
}: {
  sessionId: string;
  desktop: boolean;
  shortcut?: ReactNode;
  tasks: readonly TaskSummary[];
  backgroundActivity?: number;
}): React.JSX.Element | null {
  const summary = barSummary(tasks, backgroundActivity);
  // Marks the slot so `␣G` can open the inspector even while the bar hides.
  const marker = (
    <Box
      component="span"
      hidden
      data-background-tasks-available={tasks.length > 0 ? "true" : "false"}
      data-background-tasks-session={sessionId}
    />
  );
  if (!summary.visible) return tasks.length > 0 ? marker : null;
  const parts: { key: string; text: string; color: string }[] = [];
  if (summary.working > 0) parts.push({ key: "working", text: `◐ ${String(summary.working)} working`, color: "text.secondary" });
  if (summary.notReporting > 0) parts.push({ key: "not-reporting", text: `○ ${String(summary.notReporting)} not reporting`, color: "text.secondary" });
  if (summary.needsYou > 0) parts.push({ key: "needs", text: `⚠ ${String(summary.needsYou)} needs you`, color: "warning.main" });
  if (summary.failed > 0) parts.push({ key: "failed", text: `✕ ${String(summary.failed)} failed`, color: "error.main" });
  const lead = summary.lead;
  const leadText = lead
    ? `${lead.title}${lead.subtitle ? ` ${lead.subtitle.split(" · ").at(-1) ?? ""}` : ""}`
    : null;
  return (
    <Box
      data-background-tasks-bar={sessionId}
      sx={{
        borderBottom: desktop ? 1 : 0,
        borderColor: "divider",
        mb: desktop ? 0.5 : 0,
        color: "text.primary",
      }}
    >
      {marker}
      <ButtonBase
        data-background-tasks-open
        aria-label={`Background tasks: ${parts.map((part) => part.text.replace(/^[^ ]+ /, "")).join(", ") || "agent background activity"}. Open the list`}
        aria-haspopup="dialog"
        onClick={(event): void =>
          openBackgroundTask(sessionId, lead?.key ?? null, "bar", event.currentTarget)}
        sx={{
          width: "100%",
          minWidth: 0,
          justifyContent: "flex-start",
          gap: 0.75,
          px: desktop ? 1 : 1.5,
          minHeight: desktop ? desktopSize(32) : mobileComposerPanelHeaderMinHeight,
          borderRadius: 1,
          textAlign: "left",
          "&:hover, &:focus-visible": { bgcolor: "action.hover" },
        }}
      >
        <Typography
          variant="caption"
          noWrap
          component="span"
          sx={{ display: "flex", gap: 0.75, flexShrink: 0, fontWeight: 650 }}
        >
          {parts.map((part, index) => (
            <Box key={part.key} component="span" data-bar-part={part.key} sx={{ color: part.color }}>
              {index > 0 ? "· " : ""}{part.text}
            </Box>
          ))}
          {summary.extraActivity && (
            <Box
              component="span"
              data-bar-part="activity"
              title="The agent reports background activity that is not listed as a task"
              sx={{ color: "text.secondary" }}
            >
              {parts.length > 0 ? "· " : ""}+ background activity
            </Box>
          )}
        </Typography>
        {leadText && (
          <Typography
            variant="caption"
            noWrap
            component="span"
            data-bar-lead={lead?.key}
            sx={{ flex: 1, minWidth: 0, color: "text.secondary" }}
          >
            — {leadText}
          </Typography>
        )}
        {!leadText && <Box sx={{ flex: 1 }} />}
        {shortcut}
        <ChevronRight fontSize="small" sx={{ color: "text.secondary", flexShrink: 0 }} />
      </ButtonBase>
    </Box>
  );
}
