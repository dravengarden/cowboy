// One compact transcript row per background task anchor (a subagent child or
// an accepted Call): icon, title, subtitle and a chip that is always the last
// element. Opening details never expands the transcript — they open in the
// background-tasks inspector (Desktop Dialog, Mobile Sheet). The row keeps a
// minimum height, not a fixed one, so text scaling may grow it.

import { memo, useState } from "react";
import { Box, ButtonBase, Chip, IconButton, Paper, Typography } from "@mui/material";
import { alpha, type Theme } from "@mui/material/styles";
import { AccountTreeOutlined, CallSplitOutlined, StopCircleOutlined } from "@mui/icons-material";
import type { TaskSummary } from "./backgroundTasks";
import { openBackgroundTask } from "./backgroundTaskInspectorState";
import { DESKTOP_INSET_RADIUS } from "./desktop/DesktopEmbeddedControl";
import { Kbd } from "./Kbd";
import { SubagentStopConfirm, taskChipColor, taskToneColor } from "./SubagentCard";
import { glyphSize } from "./surface/desktopSize";
import { SUBAGENT_STOP_KEY, type DisplayGroup, type SubagentTone } from "./subagents";
import { requestTaskStop, STOP_MESSAGE, type StopState } from "./useBackgroundTasks";

export function taskTone(group: DisplayGroup): SubagentTone {
  switch (group) {
    case "active":
    case "stopping":
      return "working";
    case "waiting":
    case "needs_you":
      return "waiting";
    case "done":
      return "done";
    case "failed":
      return "failed";
    case "stopped":
      return "stopped";
    case "ended":
    case "not_reporting":
    case "unavailable":
      return "unknown";
  }
}

export function TaskChip(
  { task, size = "small" }: { task: Pick<TaskSummary, "chip" | "group" | "childAttention">; size?: "small" | "medium" },
): React.JSX.Element {
  return (
    <Box sx={{ position: "relative", display: "inline-flex", flexShrink: 0 }}>
      <Chip
        data-task-chip
        size={size}
        variant="outlined"
        color={taskChipColor(taskTone(task.group))}
        label={task.chip}
      />
      {task.childAttention > 0 && (
        <Box
          data-task-child-attention={task.childAttention}
          aria-label={`${String(task.childAttention)} nested task${task.childAttention === 1 ? "" : "s"} need attention`}
          sx={{
            position: "absolute",
            top: -2,
            right: -2,
            width: 8,
            height: 8,
            borderRadius: "50%",
            bgcolor: "warning.main",
          }}
        />
      )}
    </Box>
  );
}

function stopLine(stop: StopState | undefined): string | null {
  if (!stop || stop.outcome === "stopping") return null;
  return stop.message ? `${STOP_MESSAGE[stop.outcome]}: ${stop.message}` : STOP_MESSAGE[stop.outcome];
}

export const TaskRow = memo(function TaskRow({
  task,
  sessionId,
  desktop,
  stop,
  stopSender,
}: {
  task: TaskSummary;
  sessionId: string;
  desktop: boolean;
  stop?: StopState | undefined;
  /** Fixtures capture a subagent Stop instead of sending it. */
  stopSender?: ((sessionId: string, child: string) => boolean) | undefined;
}): React.JSX.Element {
  const [confirming, setConfirming] = useState(false);
  const [focused, setFocused] = useState(false);
  const tone = taskTone(task.group);
  const subtitle = stopLine(stop) ?? task.subtitle;
  const nested = task.source === "subagent" && task.parentKey !== null;
  const Icon = task.source === "call" ? CallSplitOutlined : AccountTreeOutlined;
  const confirmStop = (): void => {
    setConfirming(false);
    requestTaskStop(sessionId, task, stopSender);
  };
  return (
    <Paper
      elevation={0}
      data-task-row={task.key}
      data-task-group={task.group}
      {...(task.source === "subagent" && task.subagent
        ? {
          "data-subagent-row": task.subagent.id,
          "data-subagent-state": task.subagent.state,
          "data-subagent-reporting": task.subagent.reporting ? undefined : "false",
        }
        : {})}
      {...(task.call ? { "data-call-row": task.call.call_id } : {})}
      onFocus={(): void => setFocused(true)}
      onBlur={(event: React.FocusEvent<HTMLElement>): void => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false);
      }}
      sx={(theme: Theme) => ({
        alignSelf: "stretch",
        display: "flex",
        alignItems: "center",
        gap: 0.75,
        pr: 1,
        minHeight: desktop ? 40 : 44,
        overflow: "hidden",
        bgcolor: "background.paper",
        backgroundImage: `linear-gradient(105deg, ${alpha(
          taskToneColor(tone, theme),
          theme.palette.mode === "dark" ? 0.12 : 0.06,
        )}, transparent 52%)`,
        boxShadow: `0 1px 3px ${alpha(
          theme.palette.common.black,
          theme.palette.mode === "dark" ? 0.16 : 0.06,
        )}`,
        borderRadius: desktop ? `${DESKTOP_INSET_RADIUS}px` : 1,
      })}
    >
      <ButtonBase
        data-desktop-item-action="default"
        data-task-open
        {...(desktop ? { "data-desktop-widget-toggle": "task" } : {})}
        aria-haspopup="dialog"
        aria-expanded={false}
        aria-label={`${task.title}: ${task.chip}. Open details`}
        onClick={(event): void =>
          openBackgroundTask(sessionId, task.key, "row", event.currentTarget)}
        sx={{
          flex: 1,
          minWidth: 0,
          alignSelf: "stretch",
          justifyContent: "flex-start",
          textAlign: "left",
          gap: 1,
          pl: 1,
          py: 0.5,
          borderRadius: "inherit",
          "&:focus-visible": { bgcolor: "action.hover" },
        }}
      >
        <Icon sx={(theme) => ({ fontSize: glyphSize(18), color: taskToneColor(tone, theme), flexShrink: 0 })} />
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography
            variant="body2"
            noWrap
            sx={{ fontSize: "inherit", fontWeight: 650, fontFamily: "var(--cowboy-reading-font, inherit)" }}
          >
            {nested ? "↳ " : ""}
            {task.title}
            {task.childCount > 0 && (
              <Box component="span" data-task-children sx={{ color: "text.secondary", fontWeight: 500 }}>
                {` (+${String(task.childCount)})`}
              </Box>
            )}
          </Typography>
          {subtitle && (
            <Typography
              variant="caption"
              noWrap
              data-task-subtitle
              sx={{ display: "block", color: stopLine(stop) ? "warning.main" : "text.secondary" }}
            >
              {subtitle}
            </Typography>
          )}
        </Box>
      </ButtonBase>
      {task.health && (
        <Typography variant="caption" data-task-health sx={{ color: "warning.main", flexShrink: 0 }}>
          {task.health}
        </Typography>
      )}
      {task.canStop && (
        <IconButton
          size="small"
          color="error"
          data-subagent-stop
          data-task-stop
          disabled={stop?.outcome === "stopping"}
          aria-label={`Stop ${task.source === "call" ? "call" : "subagent"} ${task.title}`}
          onClick={(): void => setConfirming(true)}
          sx={{ flexShrink: 0, minWidth: { xs: 40, sm: 30 }, minHeight: { xs: 40, sm: 30 } }}
        >
          <StopCircleOutlined fontSize="small" />
          {desktop && focused && <Kbd keys={SUBAGENT_STOP_KEY} variant="context" />}
        </IconButton>
      )}
      <SubagentStopConfirm
        open={confirming && task.canStop}
        title={task.title}
        noun={task.source === "call" ? "call" : "subagent"}
        onClose={(): void => setConfirming(false)}
        onConfirm={confirmStop}
      />
      <TaskChip task={task} />
    </Paper>
  );
});

/** The anchor row of a Call the ledger has not answered for yet. It keeps the
 *  row's geometry so the first poll does not reflow the transcript. */
export function PendingCallRow({ desktop }: { desktop: boolean }): React.JSX.Element {
  return (
    <Paper
      elevation={0}
      data-call-row-pending
      sx={{
        alignSelf: "stretch",
        minHeight: desktop ? 40 : 44,
        display: "flex",
        alignItems: "center",
        gap: 1,
        px: 1,
        color: "text.secondary",
        bgcolor: "background.paper",
        borderRadius: desktop ? `${DESKTOP_INSET_RADIUS}px` : 1,
      }}
    >
      <CallSplitOutlined sx={{ fontSize: glyphSize(18) }} />
      <Typography variant="body2" sx={{ flex: 1 }}>Call</Typography>
      <Chip size="small" variant="outlined" label="Loading" />
    </Paper>
  );
}
