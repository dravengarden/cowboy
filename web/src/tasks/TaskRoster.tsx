// Real parentKey tree roster shared by Desktop Work Pane Tasks and Mobile Triage Peek.

import { useMemo } from "react";
import {
  alpha,
  Box,
  Button,
  ButtonBase,
  Chip,
  IconButton,
  Stack,
  Typography,
} from "@mui/material";
import {
  AccountTreeOutlined,
  CallSplitOutlined,
  ChevronRight,
  ExpandMore,
  PendingActionsOutlined,
} from "@mui/icons-material";
import {
  TASK_SECTION_LABEL,
  type TaskSection,
  type TaskSummary,
} from "../backgroundTasks";
import { CallCleanup } from "../CallCleanup";
import type { ManagedCallSummary } from "../managedCalls";
import { taskToneColor } from "../SubagentCard";
import { glyphSize } from "../surface/desktopSize";
import { TaskChip, taskTone } from "../TaskRow";
import {
  buildTaskForest,
  flattenTaskSections,
  type FlatTaskRow,
  taskHeadline,
  taskTimeLabel,
} from "./taskTree";

function sourceIcon(source: TaskSummary["source"]) {
  if (source === "call") return CallSplitOutlined;
  if (source === "approval") return PendingActionsOutlined;
  return AccountTreeOutlined;
}

function RosterRowContent({
  row,
  selected,
  desktop,
  now,
  onSelect,
}: {
  row: FlatTaskRow;
  selected: boolean;
  desktop: boolean;
  now: number;
  onSelect: (key: string) => void;
}): React.JSX.Element {
  const { task, collapsed, descendantCount, descendantAttention } = row;
  const Icon = sourceIcon(task.source);
  const tone = taskTone(task.group);
  const headline = taskHeadline(task);
  const elapsed = taskTimeLabel(task, now);
  return (
    <ButtonBase
      role="option"
      aria-selected={selected}
      data-task-option={task.key}
      data-desktop-item={desktop ? "true" : undefined}
      data-desktop-current={desktop && selected ? "true" : undefined}
      onClick={(): void => onSelect(task.key)}
      sx={{
        width: "100%",
        minWidth: 0,
        display: "flex",
        flexDirection: "column",
        alignItems: "stretch",
        gap: 0.35,
        px: desktop ? 1 : 1.25,
        py: desktop ? 0.65 : 0.9,
        minHeight: desktop ? 44 : 54,
        borderRadius: "inherit",
        textAlign: "left",
        "&:hover, &:focus-visible": { bgcolor: "action.hover" },
      }}
    >
      <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
        <Icon sx={(theme) => ({ fontSize: glyphSize(16), color: taskToneColor(tone, theme), flexShrink: 0 })} />
        <Typography variant="body2" noWrap sx={{ flex: 1, minWidth: 0, fontWeight: selected ? 700 : 620, fontSize: desktop ? "0.8125rem" : "0.875rem" }}>
          {task.title}
          {collapsed && descendantCount > 0 && (
            <Box component="span" sx={{ color: "text.secondary", fontWeight: 500 }}>{` (+${String(descendantCount)})`}</Box>
          )}
        </Typography>
        {collapsed && descendantAttention > 0 && (
          <Box component="span" data-task-folded-attention={descendantAttention} aria-label={`${String(descendantAttention)} nested task needs attention`} sx={{ width: 7, height: 7, borderRadius: "50%", bgcolor: "warning.main", flexShrink: 0 }} />
        )}
        {elapsed && (
          <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0, fontVariantNumeric: "tabular-nums" }}>
            {elapsed}
          </Typography>
        )}
        <TaskChip task={task} />
      </Stack>
      <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0, pl: 2.75 }}>
        <Typography variant="caption" noWrap data-task-headline sx={{ flex: 1, minWidth: 0, color: task.group === "failed" ? "error.main" : task.group === "needs_you" ? "warning.main" : "text.secondary" }}>
          {headline}
        </Typography>
        {task.health && <Typography variant="caption" color="warning.main" sx={{ flexShrink: 0 }}>{task.health}</Typography>}
      </Stack>
    </ButtonBase>
  );
}

function RosterRow({
  row,
  selected,
  cursor,
  desktop,
  now,
  onSelect,
  onToggleFold,
  onReviewRequest,
}: {
  row: FlatTaskRow;
  selected: boolean;
  cursor: boolean;
  desktop: boolean;
  now: number;
  onSelect: (key: string) => void;
  onToggleFold: (key: string) => void;
  onReviewRequest?: ((task: TaskSummary) => void) | undefined;
}): React.JSX.Element {
  const { task, depth, hasChildren, collapsed } = row;
  const actionableRequest = !desktop && (task.source === "approval" || task.group === "needs_you");
  const chevronSlot = desktop ? 24 : 32;
  const step = desktop ? 18 : 16;
  const indent = depth === 0 ? 0 : (hasChildren ? depth * step : chevronSlot + depth * step);
  const guideLeft = Math.max(8, indent - 10);
  return (
    <Box
      data-task-tree-row={task.key}
      data-task-depth={depth}
      sx={{
        display: "flex",
        alignItems: "stretch",
        pl: indent > 0 ? `${String(indent)}px` : 0,
        position: "relative",
        ...(depth > 0 && {
          "&::before": { content: '""', position: "absolute", left: `${String(guideLeft)}px`, top: 4, bottom: 4, width: "1px", bgcolor: "divider" },
        }),
      }}
    >
      {hasChildren && (
        <IconButton
          size="small"
          aria-label={collapsed ? `Expand ${task.title}` : `Collapse ${task.title}`}
          data-task-fold={task.key}
          onClick={(event): void => {
            event.stopPropagation();
            onToggleFold(task.key);
          }}
          sx={{ alignSelf: "center", width: chevronSlot - 2, height: chevronSlot - 2, mr: 0.25, flexShrink: 0, color: "text.secondary" }}
        >
          {collapsed ? <ChevronRight fontSize="inherit" /> : <ExpandMore fontSize="inherit" />}
        </IconButton>
      )}
      <Box
        sx={(theme) => ({
          flex: 1,
          minWidth: 0,
          display: "flex",
          flexDirection: "column",
          borderRadius: desktop ? 1 : 1.5,
          border: desktop ? 0 : 1,
          borderColor: selected ? "primary.main" : task.group === "needs_you" ? alpha(theme.palette.warning.main, 0.45) : task.attention === "failed_unseen" ? alpha(theme.palette.error.main, 0.4) : "divider",
          bgcolor: selected ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.16 : 0.1) : cursor ? "action.hover" : desktop ? "transparent" : alpha(theme.palette.background.paper, theme.palette.mode === "dark" ? 0.52 : 0.72),
          outline: cursor && !selected ? `1px solid ${alpha(theme.palette.primary.main, 0.5)}` : "none",
          transition: "background-color 100ms ease",
        })}
      >
        <RosterRowContent row={row} selected={selected} desktop={desktop} now={now} onSelect={onSelect} />
        {actionableRequest && onReviewRequest && (
          <Box sx={{ px: 1.25, pb: 0.85, pt: 0.1, display: "flex", justifyContent: "flex-end" }}>
            <Button size="small" variant="outlined" color="warning" data-task-review-request={task.key} onClick={(event): void => { event.stopPropagation(); onReviewRequest(task); }} sx={{ textTransform: "none", minHeight: 34, fontSize: "0.75rem", fontWeight: 650 }}>
              Review request
            </Button>
          </Box>
        )}
      </Box>
    </Box>
  );
}

export function TaskRoster({
  sessionId,
  tasks,
  calls,
  selectedKey,
  cursorKey,
  desktop,
  hideEnded,
  onToggleHideEnded,
  collapsedKeys,
  onToggleFold,
  onSelect,
  onRefreshCalls,
  onReviewRequest,
  now = Date.now(),
}: {
  sessionId: string;
  tasks: readonly TaskSummary[];
  calls: readonly ManagedCallSummary[];
  selectedKey: string | null;
  cursorKey?: string | null;
  desktop: boolean;
  hideEnded: boolean;
  onToggleHideEnded: () => void;
  collapsedKeys: ReadonlySet<string>;
  onToggleFold: (key: string) => void;
  onSelect: (key: string) => void;
  onRefreshCalls: () => void;
  onReviewRequest?: ((task: TaskSummary) => void) | undefined;
  now?: number;
}): React.JSX.Element {
  const forest = useMemo(() => buildTaskForest(tasks), [tasks]);
  const flatRows = useMemo(() => flattenTaskSections(forest, { collapsedKeys, hideEnded }), [forest, collapsedKeys, hideEnded]);
  const groupedRows = useMemo(() => {
    const bySection = new Map<TaskSection, FlatTaskRow[]>();
    for (const row of flatRows) {
      const list = bySection.get(row.section) ?? [];
      list.push(row);
      bySection.set(row.section, list);
    }
    return (["needs_you", "active", "ended"] as const)
      .map((section) => ({ section, rows: bySection.get(section) ?? [] }))
      .filter((entry) => entry.rows.length > 0);
  }, [flatRows]);

  const hasEnded = useMemo(() => tasks.some((task) => task.group === "done" || task.group === "failed" || task.group === "stopped" || task.group === "ended" || task.group === "unavailable"), [tasks]);
  const hasEndedCalls = useMemo(() => calls.some((call) => call.state === "completed" || call.state === "failed" || call.state === "cancelled"), [calls]);

  return (
    <Stack spacing={1} sx={{ minWidth: 0, p: desktop ? 1 : 0 }}>
      {(hasEnded || hasEndedCalls) && (
        <Stack direction="row" spacing={0.75} alignItems="center" flexWrap="wrap" useFlexGap sx={{ px: desktop ? 0.5 : 0 }}>
          {hasEnded && (
            <Chip size="small" clickable data-task-hide-ended variant={hideEnded ? "filled" : "outlined"} color={hideEnded ? "primary" : "default"} label={hideEnded ? "Showing active" : "Hide ended"} onClick={onToggleHideEnded} sx={{ height: desktop ? 24 : 28, fontSize: "0.75rem" }} />
          )}
          {hasEndedCalls && <Box sx={{ ml: "auto" }}><CallCleanup parent={sessionId} calls={calls} onChange={onRefreshCalls} /></Box>}
        </Stack>
      )}
      {groupedRows.length === 0
        ? (
          <Typography variant="body2" color="text.secondary" sx={{ p: 1.5 }}>
            {hideEnded && tasks.length > 0 ? "All tasks have ended. Clear the filter to view history." : "No background tasks in this session."}
          </Typography>
        )
        : (
          <Stack component="nav" aria-label="Background tasks" spacing={1.25}>
            {groupedRows.map(({ section, rows }) => (
              <Box key={section} data-task-section={section}>
                <Typography variant="caption" sx={{ display: "block", px: 0.75, pb: 0.4, color: section === "needs_you" ? "warning.main" : "text.secondary", fontWeight: 700, letterSpacing: "0.03em" }}>
                  {TASK_SECTION_LABEL[section]}
                </Typography>
                <Stack spacing={desktop ? 0.25 : 0.75} role="listbox" aria-label={TASK_SECTION_LABEL[section]}>
                  {rows.map((row) => (
                    <RosterRow key={row.task.key} row={row} selected={row.task.key === selectedKey} cursor={row.task.key === (cursorKey ?? null)} desktop={desktop} now={now} onSelect={onSelect} onToggleFold={onToggleFold} onReviewRequest={onReviewRequest} />
                  ))}
                </Stack>
              </Box>
            ))}
          </Stack>
        )}
    </Stack>
  );
}
