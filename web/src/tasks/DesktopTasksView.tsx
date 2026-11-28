// Non-modal Desktop Work Pane tab (`Conversation ␣C | Review ␣V | Tasks ␣G`).

import { ScrollEdges } from "@cowboy/app-shell";
import { useCallback, useEffect, useRef, useState } from "react";
import { alpha, Box, ButtonBase, Chip, Stack, Typography } from "@mui/material";
import { FormatListBulleted } from "@mui/icons-material";
import type { TaskSummary } from "../backgroundTasks";
import { closeBackgroundTasks, inspectorState } from "../backgroundTaskInspectorState";
import { setActiveSessionId } from "../controlPlane";
import { desktopScrollbarSx } from "../desktop/desktopScrollbar";
import { useDesktopWorkspace } from "../desktop/DesktopWorkspaceController";
import { requestDesktopWorkView } from "../desktop/review/desktopWorkView";
import { callConversationId } from "../managedCalls";
import { useSubagentTranscript } from "../SubagentTranscript";
import { taskToneColor } from "../SubagentCard";
import { taskTone } from "../TaskRow";
import type { BackgroundTasks } from "../useBackgroundTasks";
import { LegacyCallsSwitch } from "./TaskDiagnostics";
import { dispatchLocateTask, dispatchOpenTaskOriginTool, focusTaskApprovalSurface, quoteTaskToPrompt } from "./taskEvents";
import { defaultReaderMode, type TaskReaderContentMode, type TaskReaderViewMode, TaskResultReader } from "./TaskResultReader";
import { TaskRoster } from "./TaskRoster";
import { taskPillLabel } from "./taskTree";
import { useDesktopTasksCommands, useDesktopTasksController, useDesktopTasksPaneKeyHandler, useDesktopTasksPaneWidth } from "./useDesktopTasksController";

const SPLIT_MIN_WIDTH_PX = 900;

function DesktopTasksCompactStrip({
  totalCount,
  visibleTasks,
  selectedKey,
  showRoster,
  onToggleRoster,
  onSelect,
}: {
  totalCount: number;
  visibleTasks: readonly TaskSummary[];
  selectedKey: string | null;
  showRoster: boolean;
  onToggleRoster: () => void;
  onSelect: (key: string) => void;
}): React.JSX.Element {
  return (
    <Stack direction="row" spacing={0.5} alignItems="center" data-desktop-tasks-strip sx={{ px: 1, py: 0.6, borderBottom: 1, borderColor: "divider", flexShrink: 0, overflowX: "auto", scrollbarWidth: "none", "&::-webkit-scrollbar": { display: "none" } }}>
      <Chip size="small" clickable data-desktop-tasks-all icon={<FormatListBulleted fontSize="inherit" />} label={`All ${String(totalCount)}`} variant={showRoster ? "filled" : "outlined"} color={showRoster ? "primary" : "default"} onClick={onToggleRoster} sx={{ height: 24, flexShrink: 0, fontWeight: 650, fontSize: "0.75rem" }} />
      {visibleTasks.map((task) => {
        const isSelected = !showRoster && selectedKey === task.key;
        const tone = taskTone(task.group);
        return (
          <ButtonBase key={task.key} data-desktop-task-pill={task.key} aria-pressed={isSelected} title={`${task.title} · ${task.chip}`} onClick={(): void => onSelect(task.key)} sx={(theme) => ({ flexShrink: 0, maxWidth: 168, height: 24, px: 1, gap: 0.6, borderRadius: 999, fontSize: "0.75rem", fontWeight: isSelected ? 700 : 550, border: 1, borderColor: isSelected ? "primary.main" : "divider", bgcolor: isSelected ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.2 : 0.12) : "transparent", color: isSelected ? "text.primary" : "text.secondary", "&:hover": { bgcolor: "action.hover", color: "text.primary" } })}>
            <Box component="span" sx={(theme) => ({ width: 6, height: 6, borderRadius: "50%", flexShrink: 0, bgcolor: taskToneColor(tone, theme) })} />
            <Typography variant="caption" noWrap sx={{ fontWeight: "inherit", fontSize: "inherit" }}>{taskPillLabel(task)}</Typography>
          </ButtonBase>
        );
      })}
    </Stack>
  );
}

function focusedRosterTask(
  byKey: ReadonlyMap<string, TaskSummary>,
  fallback: TaskSummary | null,
): TaskSummary | null {
  const activeEl = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const optionKey = activeEl?.closest<HTMLElement>("[data-desktop-region='tasks.roster'] [data-task-option]")?.dataset["taskOption"];
  return (optionKey ? byKey.get(optionKey) : undefined) ?? fallback;
}

function DesktopTasksPanes({
  sessionId,
  wide,
  showRosterPane,
  showReaderPane,
  effectiveMode,
  selected,
  background,
  subagentTranscript,
  summaryExtraActivity,
  hideEnded,
  collapsedKeys,
  now,
  stopTrigger,
  legacySwitch,
  onToggleHideEnded,
  onToggleFold,
  onSelect,
  onModeChange,
  onAnswerApproval,
  onLocate,
  onOpenOriginTool,
  onOpenChildSession,
  onQuoteRequest,
  onStopTriggerConsumed,
}: {
  sessionId: string;
  wide: boolean;
  showRosterPane: boolean;
  showReaderPane: boolean;
  effectiveMode: TaskReaderViewMode;
  selected: TaskSummary | null;
  background: BackgroundTasks;
  subagentTranscript: ReturnType<typeof useSubagentTranscript>;
  summaryExtraActivity: boolean;
  hideEnded: boolean;
  collapsedKeys: ReadonlySet<string>;
  now: number;
  stopTrigger: { key: string; nonce: number } | null;
  legacySwitch: React.JSX.Element;
  onToggleHideEnded: () => void;
  onToggleFold: (key: string) => void;
  onSelect: (key: string) => void;
  onModeChange: (next: TaskReaderViewMode) => void;
  onAnswerApproval: (task: TaskSummary) => void;
  onLocate: (task: TaskSummary) => void;
  onOpenOriginTool: (toolKey: string) => void;
  onOpenChildSession: (childId: string) => void;
  onQuoteRequest: (task: TaskSummary) => void;
  onStopTriggerConsumed: () => void;
}): React.JSX.Element {
  return (
    <Box sx={{ flex: 1, minHeight: 0, minWidth: 0, display: "flex", overflow: "hidden" }}>
      {showRosterPane && (
        <Box data-desktop-region="tasks.roster" data-desktop-focus-default={!showReaderPane ? "true" : undefined} tabIndex={-1} sx={{ ...(wide ? { width: 236, flexShrink: 0, borderRight: 1, borderColor: "divider" } : { flex: 1 }), minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden", outline: "none" }}>
          <Box sx={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", overflowY: "auto", ...desktopScrollbarSx }}>
            <TaskRoster sessionId={sessionId} tasks={background.tasks} calls={background.calls} selectedKey={selected?.key ?? null} desktop hideEnded={hideEnded} onToggleHideEnded={onToggleHideEnded} collapsedKeys={collapsedKeys} onToggleFold={onToggleFold} onSelect={onSelect} onRefreshCalls={background.refreshCalls} now={now} />
            <Box sx={{ flex: 1 }} />
            {legacySwitch}
            <ScrollEdges />
          </Box>
        </Box>
      )}
      {showReaderPane && (
        <Box data-desktop-region="tasks.reader" data-desktop-navigation={effectiveMode === "transcript" ? "scroll" : undefined} data-desktop-focus-default="true" tabIndex={-1} sx={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden", outline: "none" }}>
          {selected
            ? <TaskResultReader sessionId={sessionId} task={selected} background={background} subagentTranscript={subagentTranscript} desktop mode={effectiveMode} onModeChange={onModeChange} onAnswerApproval={(): void => onAnswerApproval(selected)} onLocate={selected.anchorSeq !== null ? (): void => onLocate(selected) : undefined} onOpenOriginTool={onOpenOriginTool} onOpenChildSession={onOpenChildSession} onQuoteRequest={(): void => onQuoteRequest(selected)} stopTrigger={stopTrigger} onStopTriggerConsumed={onStopTriggerConsumed} extraInfoFooter={!wide ? legacySwitch : undefined} />
            : <Box sx={{ p: 3, color: "text.secondary" }}><Typography variant="body2">{summaryExtraActivity ? "The agent reports background activity that is not listed as an individual task yet." : "No background tasks in this session."}</Typography></Box>}
        </Box>
      )}
    </Box>
  );
}

function useTasksSubpaneFocusRecovery(
  active: boolean,
  showReaderPane: boolean,
  showRosterPane: boolean,
  rootRef: React.RefObject<HTMLDivElement | null>,
  focusedRegion: string | null,
  focusRegion: (region: string) => void,
): void {
  const prevRef = useRef({ showReaderPane, showRosterPane });
  useEffect(() => {
    const prev = prevRef.current;
    prevRef.current = { showReaderPane, showRosterPane };
    const subpaneHidden = (prev.showReaderPane && !showReaderPane) || (prev.showRosterPane && !showRosterPane);
    if (!active || !subpaneHidden || (focusedRegion !== "tasks.reader" && focusedRegion !== "tasks.roster")) return;
    const node = rootRef.current;
    const activeEl = document.activeElement;
    if (!node || (activeEl && activeEl !== document.body && node.contains(activeEl))) return;
    const target = showReaderPane ? "tasks.reader" : showRosterPane ? "tasks.roster" : null;
    if (!target) return;
    const frame = requestAnimationFrame(() => {
      const live = document.activeElement;
      if (live && live !== document.body && live.closest("[data-desktop-region]") !== null) return;
      focusRegion(target);
    });
    return () => cancelAnimationFrame(frame);
  }, [active, focusedRegion, focusRegion, rootRef, showReaderPane, showRosterPane]);
}

export function DesktopTasksView({
  sessionId,
  active,
  providedBackground,
  onLocate,
  onOpenOriginTool,
}: {
  sessionId: string;
  active: boolean;
  providedBackground?: BackgroundTasks | undefined;
  onLocate?: ((key: string) => void) | undefined;
  onOpenOriginTool?: ((toolKey: string) => void) | undefined;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const rootRef = useRef<HTMLDivElement>(null);
  const paneWidth = useDesktopTasksPaneWidth(rootRef, active);
  const ctrl = useDesktopTasksController(sessionId, active, providedBackground);
  const { background, summary, visibleTasks, selected } = ctrl;
  const subagentTranscript = useSubagentTranscript(sessionId, active && selected?.source === "subagent" ? (selected.subagent ?? null) : null);
  const hasConversation = Boolean(selected && (selected.source === "call" || (selected.source === "subagent" && subagentTranscript.observed)));
  const wide = paneWidth >= SPLIT_MIN_WIDTH_PX;
  const showRosterPane = wide || ctrl.compactShowRoster || !selected;
  const showReaderPane = wide || (!ctrl.compactShowRoster && Boolean(selected));
  const fallbackMode: TaskReaderContentMode = selected ? defaultReaderMode(selected, subagentTranscript.observed) : "result";
  const effectiveMode: TaskReaderViewMode = ctrl.readerMode && selected && ctrl.readerMode.key === selected.key ? ctrl.readerMode.mode : fallbackMode;

  const setDesktopMode = useCallback((next: TaskReaderViewMode): void => {
    if (!selected) return;
    ctrl.setReaderMode((prev) => {
      const prevContent: TaskReaderContentMode = prev && prev.key === selected.key ? (prev.mode === "info" ? prev.prev : prev.mode) : fallbackMode;
      return { key: selected.key, mode: next, prev: next === "info" ? prevContent : next };
    });
  }, [ctrl, fallbackMode, selected]);

  useTasksSubpaneFocusRecovery(active, showReaderPane, showRosterPane, rootRef, workspace.focusedRegion, workspace.focusRegion);

  const [stopTrigger, setStopTrigger] = useState<{ key: string; nonce: number } | null>(null);
  const mountedRef = useRef(true);
  useEffect(() => () => { mountedRef.current = false; }, []);
  const clearStopTrigger = useCallback((): void => setStopTrigger(null), []);
  const focusedOrSelectedTask = useCallback(
    (): TaskSummary | null => focusedRosterTask(background.byKey, selected),
    [background.byKey, selected],
  );

  const handleLocateTask = useCallback((task: TaskSummary): void => {
    if (task.anchorSeq === null) return;
    requestDesktopWorkView("conversation");
    if (onLocate) onLocate(task.key);
    else dispatchLocateTask(sessionId, task.key);
  }, [onLocate, sessionId]);
  const handleOpenOriginTool = useCallback((toolKey: string): void => {
    requestDesktopWorkView("conversation");
    if (onOpenOriginTool) onOpenOriginTool(toolKey);
    else dispatchOpenTaskOriginTool(sessionId, toolKey);
  }, [onOpenOriginTool, sessionId]);
  const handleOpenChildSession = useCallback((childId: string): void => {
    if (inspectorState().sessionId === sessionId) closeBackgroundTasks();
    setActiveSessionId(childId);
  }, [sessionId]);
  const handleAnswerApproval = useCallback((task: TaskSummary): void => { focusTaskApprovalSurface(sessionId, task); }, [sessionId]);
  const chooseAndFocus = useCallback((key: string): void => {
    setStopTrigger(null);
    const wasCompactRoster = !wide && ctrl.compactShowRoster;
    ctrl.chooseTask(key);
    if (wasCompactRoster) requestAnimationFrame(() => workspace.focusRegion("tasks.reader"));
  }, [ctrl, wide, workspace]);
  const stepAndFocus = useCallback((delta: number): void => {
    setStopTrigger(null);
    const wasCompactRoster = !wide && ctrl.compactShowRoster;
    ctrl.stepTask(delta);
    if (wasCompactRoster) requestAnimationFrame(() => workspace.focusRegion("tasks.reader"));
  }, [ctrl, wide, workspace]);

  const liveOwnerRef = useRef({ active, sessionId, key: selected?.key ?? null });
  liveOwnerRef.current = { active, sessionId, key: selected?.key ?? null };
  const revealReader = useCallback((): void => {
    if (!wide && ctrl.compactShowRoster) {
      ctrl.setCompactShowRoster(false);
      requestAnimationFrame(() => workspace.focusRegion("tasks.reader"));
    }
  }, [ctrl, wide, workspace]);
  const handleQuoteTask = useCallback((task: TaskSummary): void => {
    const reqSession = sessionId;
    const reqKey = task.key;
    quoteTaskToPrompt(sessionId, task, () => {
      const live = liveOwnerRef.current;
      const focusedKey = focusedOrSelectedTask()?.key ?? null;
      if (mountedRef.current && live.active && live.sessionId === reqSession && (live.key === reqKey || focusedKey === reqKey)) {
        requestAnimationFrame(() => {
          if (mountedRef.current) workspace.focusRegion("prompt.composer");
        });
      }
    });
  }, [focusedOrSelectedTask, sessionId, workspace]);
  const restoreFromInfo = (): void => {
    if (selected) setDesktopMode(ctrl.readerMode && ctrl.readerMode.key === selected.key ? ctrl.readerMode.prev : fallbackMode);
  };
  const openModeForTask = (task: TaskSummary, mode: "info" | "transcript"): void => {
    if (task.key !== selected?.key) {
      ctrl.chooseTask(task.key);
      ctrl.setReaderMode({ key: task.key, mode, prev: mode === "transcript" ? "transcript" : defaultReaderMode(task, false) });
      revealReader();
      return;
    }
    if (mode === "transcript" && !hasConversation) return;
    revealReader();
    if (mode === "info") { if (effectiveMode === "info" && showReaderPane) restoreFromInfo(); else setDesktopMode("info"); }
    else setDesktopMode(effectiveMode === "transcript" && showReaderPane ? "result" : "transcript");
  };
  useDesktopTasksCommands({
    active,
    getTarget: focusedOrSelectedTask,
    canTranscript: (task) => task.source === "call" || (task.source === "subagent" && (task.key === selected?.key ? hasConversation : Boolean(task.subagent))),
    effectiveMode,
    hideEnded: ctrl.hideEnded,
    stepTask: stepAndFocus,
    toggleInfo: (task): void => openModeForTask(task, "info"),
    toggleTranscript: (task): void => openModeForTask(task, "transcript"),
    quote: handleQuoteTask,
    openChild: (task): void => { if (task.source === "call" && task.call) handleOpenChildSession(callConversationId(task.call)); },
    locate: handleLocateTask,
    stop: (task): void => { if (!task.canStop) return; if (task.key !== selected?.key) ctrl.chooseTask(task.key); revealReader(); setStopTrigger((prev) => ({ key: task.key, nonce: (prev?.nonce ?? 0) + 1 })); },
    escape: (): void => { if (effectiveMode === "info") restoreFromInfo(); else if (ctrl.hideEnded) ctrl.setHideEnded(false); },
  });

  const handleKeyDown = useDesktopTasksPaneKeyHandler({ active, rootRef, byKey: background.byKey, selected, stepTask: stepAndFocus, chooseTask: chooseAndFocus, handleAnswerApproval, focusReader: () => workspace.focusRegion("tasks.reader") });
  const legacySwitch = <LegacyCallsSwitch onSwitch={(): void => { closeBackgroundTasks(); requestDesktopWorkView("conversation"); }} />;

  return (
    <Box ref={rootRef} data-desktop-tasks-view={sessionId} data-desktop-tasks-layout={wide ? "split" : "compact"} onKeyDown={handleKeyDown} sx={{ flex: 1, minHeight: 0, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
      {!wide && background.tasks.length > 0 && (
        <DesktopTasksCompactStrip totalCount={background.tasks.length} visibleTasks={visibleTasks} selectedKey={selected?.key ?? null} showRoster={ctrl.compactShowRoster} onToggleRoster={(): void => { setStopTrigger(null); const next = !ctrl.compactShowRoster; ctrl.setCompactShowRoster(next); requestAnimationFrame(() => workspace.focusRegion(next ? "tasks.roster" : "tasks.reader")); }} onSelect={chooseAndFocus} />
      )}
      <DesktopTasksPanes sessionId={sessionId} wide={wide} showRosterPane={showRosterPane} showReaderPane={showReaderPane} effectiveMode={effectiveMode} selected={selected} background={background} subagentTranscript={subagentTranscript} summaryExtraActivity={summary.extraActivity} hideEnded={ctrl.hideEnded} collapsedKeys={ctrl.collapsedKeys} now={ctrl.now} stopTrigger={stopTrigger} legacySwitch={legacySwitch} onToggleHideEnded={(): void => ctrl.setHideEnded((value) => !value)} onToggleFold={ctrl.toggleFold} onSelect={chooseAndFocus} onModeChange={setDesktopMode} onAnswerApproval={handleAnswerApproval} onLocate={handleLocateTask} onOpenOriginTool={handleOpenOriginTool} onOpenChildSession={handleOpenChildSession} onQuoteRequest={handleQuoteTask} onStopTriggerConsumed={clearStopTrigger} />
    </Box>
  );
}
