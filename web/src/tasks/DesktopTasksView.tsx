// Non-modal Desktop Work Pane tab (`Conversation ␣C | Review ␣V | Tasks ␣G`).

import { ScrollEdges } from "@cowboy/app-shell";
import { type RefObject, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { alpha, Box, ButtonBase, Chip, Stack, Typography } from "@mui/material";
import { FormatListBulleted } from "@mui/icons-material";
import { acknowledgeTask, taskKeyFromHash, type TaskSummary } from "../backgroundTasks";
import { closeBackgroundTasks, openBackgroundTask, selectBackgroundTask, useInspectorState } from "../backgroundTaskInspectorState";
import { setActiveSessionId } from "../controlPlane";
import { shortcutKey } from "../desktop/commands/imeShortcut";
import { desktopScrollbarSx } from "../desktop/desktopScrollbar";
import { useDesktopWorkspace } from "../desktop/DesktopWorkspaceController";
import { requestDesktopWorkView } from "../desktop/review/desktopWorkView";
import { callConversationId } from "../managedCalls";
import { useSubagentTranscript } from "../SubagentTranscript";
import { taskToneColor } from "../SubagentCard";
import { taskTone } from "../TaskRow";
import { type BackgroundTasks, useBackgroundTasks } from "../useBackgroundTasks";
import { LegacyCallsSwitch } from "./TaskDiagnostics";
import { clearSessionTasksAvailability, dispatchLocateTask, dispatchOpenTaskOriginTool, publishSessionTasksAvailability, registerDesktopTasksHost } from "./taskEvents";
import { TaskResultReader, type TaskReaderViewMode } from "./TaskResultReader";
import { TaskRoster } from "./TaskRoster";
import { buildTaskForest, flattenTaskSections, isTaskEnded, taskPillLabel, taskTabSummary } from "./taskTree";

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

function useDesktopTasksKeyHandler({
  active,
  rootRef,
  selected,
  currentMode,
  hideEnded,
  observedSubagent,
  stepTask,
  setCompactShowRoster,
  setReaderMode,
  setHideEnded,
  handleAnswerApproval,
  handleLocate,
  focusReader,
}: {
  active: boolean;
  rootRef: RefObject<HTMLDivElement | null>;
  selected: TaskSummary | null;
  currentMode: TaskReaderViewMode | undefined;
  hideEnded: boolean;
  observedSubagent: boolean;
  stepTask: (delta: number) => void;
  setCompactShowRoster: (value: boolean) => void;
  setReaderMode: React.Dispatch<React.SetStateAction<{ key: string; mode: TaskReaderViewMode } | null>>;
  setHideEnded: (value: boolean) => void;
  handleAnswerApproval: () => void;
  handleLocate: () => void;
  focusReader: () => void;
}): (event: React.KeyboardEvent<HTMLDivElement>) => void {
  return useCallback((event: React.KeyboardEvent<HTMLDivElement>): void => {
    if (!active || event.metaKey || event.ctrlKey || event.altKey) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest("input, textarea, select, [contenteditable='true'], .cm-editor, [role='dialog']")) return;
    const key = shortcutKey(event.nativeEvent);
    if (!key) return;
    const inRoster = Boolean(target?.closest("[data-desktop-region='tasks.roster']"));
    if (key === "[" || key === "]") { event.preventDefault(); event.stopPropagation(); stepTask(key === "]" ? 1 : -1); return; }
    if (key === "j" || key === "ArrowDown" || key === "k" || key === "ArrowUp") {
      const delta = key === "j" || key === "ArrowDown" ? 1 : -1;
      if (inRoster) { event.preventDefault(); event.stopPropagation(); stepTask(delta); return; }
      const scroller = rootRef.current?.querySelector<HTMLElement>("[data-task-reader-scroll], [data-task-transcript-host] [tabindex]");
      if (scroller) { event.preventDefault(); event.stopPropagation(); scroller.scrollBy({ top: delta * 72 }); }
      return;
    }
    if (key === "Enter" && inRoster && selected) {
      event.preventDefault(); event.stopPropagation();
      if (selected.source === "approval") handleAnswerApproval();
      else { setCompactShowRoster(false); requestAnimationFrame(focusReader); }
      return;
    }
    if (key === "i" && selected) {
      event.preventDefault(); event.stopPropagation();
      const fallback: TaskReaderViewMode = isTaskEnded(selected) ? "result" : selected.source === "call" || observedSubagent ? "transcript" : "result";
      setReaderMode((prev) => ({ key: selected.key, mode: prev?.key === selected.key && prev.mode === "info" ? fallback : "info" }));
      return;
    }
    if (key === "t" && selected && (selected.source === "call" || observedSubagent)) {
      event.preventDefault(); event.stopPropagation();
      setReaderMode((prev) => ({ key: selected.key, mode: prev?.key === selected.key && prev.mode === "transcript" ? "result" : "transcript" }));
      return;
    }
    if (key === "q" && selected) { event.preventDefault(); event.stopPropagation(); rootRef.current?.querySelector<HTMLButtonElement>("[data-task-quote-prompt]")?.click(); return; }
    if ((key === "o" || key === "O") && selected?.source === "call" && selected.call) { event.preventDefault(); event.stopPropagation(); setActiveSessionId(callConversationId(selected.call)); return; }
    if (key === "L" && selected && selected.anchorSeq !== null) { event.preventDefault(); event.stopPropagation(); handleLocate(); return; }
    if ((key === "x" || key === "X") && selected?.canStop) { event.preventDefault(); event.stopPropagation(); rootRef.current?.querySelector<HTMLButtonElement>("[data-task-detail-stop]")?.click(); return; }
    if (key === "Escape") {
      if (currentMode === "info") { event.preventDefault(); event.stopPropagation(); setReaderMode(null); }
      else if (hideEnded) { event.preventDefault(); event.stopPropagation(); setHideEnded(false); }
    }
  }, [active, currentMode, focusReader, handleAnswerApproval, handleLocate, hideEnded, observedSubagent, rootRef, selected, setCompactShowRoster, setHideEnded, setReaderMode, stepTask]);
}

function useDesktopTasksPaneWidth(rootRef: RefObject<HTMLDivElement | null>, active: boolean): number {
  const [paneWidth, setPaneWidth] = useState(720);
  useLayoutEffect(() => {
    const node = rootRef.current;
    if (!node) return undefined;
    const measure = (): void => {
      const width = node.getBoundingClientRect().width;
      if (width > 0) setPaneWidth(width);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [rootRef, active]);
  return paneWidth;
}

function useDesktopTasksController(sessionId: string, active: boolean, providedBackground?: BackgroundTasks) {
  const liveBackground = useBackgroundTasks(sessionId);
  const background = providedBackground ?? liveBackground;
  const inspector = useInspectorState();
  const [hideEnded, setHideEnded] = useState(false);
  const [collapsedKeys, setCollapsedKeys] = useState<ReadonlySet<string>>(() => new Set());
  const [compactShowRoster, setCompactShowRoster] = useState(false);
  const [readerMode, setReaderMode] = useState<{ key: string; mode: TaskReaderViewMode } | null>(null);
  const [localKey, setLocalKey] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => registerDesktopTasksHost(sessionId), [sessionId]);
  const summary = useMemo(() => taskTabSummary(background.tasks, background.backgroundActivity), [background.tasks, background.backgroundActivity]);
  useEffect(() => {
    publishSessionTasksAvailability(sessionId, {
      hasTasks: summary.hasTasks,
      activeCount: summary.activeCount + summary.notReportingCount,
      needsYouCount: summary.needsYouCount,
      failedUnseenCount: summary.failedUnseenCount,
    });
  }, [sessionId, summary]);
  useEffect(() => () => clearSessionTasksAvailability(sessionId), [sessionId]);
  useEffect(() => {
    if (!active || summary.activeCount === 0) return undefined;
    const timer = globalThis.setInterval(() => setNow(Date.now()), 5_000);
    return () => globalThis.clearInterval(timer);
  }, [active, summary.activeCount]);

  const forest = useMemo(() => buildTaskForest(background.tasks), [background.tasks]);
  const visibleTasks = useMemo(() => flattenTaskSections(forest, { collapsedKeys, hideEnded }).map((row) => row.task), [forest, collapsedKeys, hideEnded]);
  const selectedKey = useMemo(() => {
    const preferred = inspector.sessionId === sessionId && inspector.key ? inspector.key : localKey;
    if (preferred && background.byKey.has(preferred)) return preferred;
    return visibleTasks[0]?.key ?? background.tasks[0]?.key ?? null;
  }, [inspector.sessionId, inspector.key, localKey, sessionId, background.byKey, visibleTasks, background.tasks]);
  const selected = selectedKey ? background.byKey.get(selectedKey) ?? null : null;

  useEffect(() => {
    const key = taskKeyFromHash(globalThis.location?.hash ?? "");
    if (!key || !background.byKey.has(key)) return;
    setLocalKey(key);
    setCompactShowRoster(false);
    openBackgroundTask(sessionId, key, "link");
    requestDesktopWorkView("tasks");
  }, [sessionId, background.byKey]);

  useEffect(() => {
    if (inspector.sessionId !== sessionId) return;
    if (inspector.key && background.byKey.has(inspector.key)) {
      setLocalKey(inspector.key);
      setCompactShowRoster(false);
    } else if (inspector.key === null && background.tasks.length > 1) {
      setCompactShowRoster(true);
    }
  }, [inspector.revision, inspector.sessionId, inspector.key, sessionId, background.byKey, background.tasks.length]);

  const chooseTask = useCallback((key: string): void => {
    setLocalKey(key);
    setCompactShowRoster(false);
    setReaderMode(null);
    if (inspector.sessionId === sessionId) selectBackgroundTask(key);
    const picked = background.byKey.get(key);
    if (picked?.attention === "failed_unseen") acknowledgeTask(sessionId, picked);
  }, [background.byKey, inspector.sessionId, sessionId]);

  const stepTask = useCallback((delta: number): void => {
    if (visibleTasks.length === 0) return;
    const index = selected ? visibleTasks.findIndex((item) => item.key === selected.key) : -1;
    const next = visibleTasks[(index === -1 ? 0 : index + delta + visibleTasks.length) % visibleTasks.length];
    if (next) chooseTask(next.key);
  }, [chooseTask, selected, visibleTasks]);

  const toggleFold = useCallback((key: string): void => {
    setCollapsedKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  return { background, summary, visibleTasks, selected, hideEnded, setHideEnded, collapsedKeys, toggleFold, compactShowRoster, setCompactShowRoster, readerMode, setReaderMode, now, chooseTask, stepTask };
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
  const wide = paneWidth >= SPLIT_MIN_WIDTH_PX;
  const currentMode = ctrl.readerMode && selected && ctrl.readerMode.key === selected.key ? ctrl.readerMode.mode : undefined;

  const handleLocate = useCallback((): void => {
    if (!selected || selected.anchorSeq === null) return;
    requestDesktopWorkView("conversation");
    if (onLocate) onLocate(selected.key);
    else dispatchLocateTask(sessionId, selected.key);
  }, [onLocate, selected, sessionId]);

  const handleOpenOriginTool = useCallback((toolKey: string): void => {
    requestDesktopWorkView("conversation");
    if (onOpenOriginTool) onOpenOriginTool(toolKey);
    else dispatchOpenTaskOriginTool(sessionId, toolKey);
  }, [onOpenOriginTool, sessionId]);

  const handleAnswerApproval = useCallback((): void => {
    const approval = document.querySelector<HTMLElement>("[data-call-approval]");
    if (approval) {
      approval.scrollIntoView({ block: "nearest" });
      approval.querySelector<HTMLElement>("button")?.focus({ preventScroll: true });
      return;
    }
    workspace.focusRegion("conversation.approval");
  }, [workspace]);

  const handleKeyDown = useDesktopTasksKeyHandler({
    active,
    rootRef,
    selected,
    currentMode,
    hideEnded: ctrl.hideEnded,
    observedSubagent: subagentTranscript.observed,
    stepTask: ctrl.stepTask,
    setCompactShowRoster: ctrl.setCompactShowRoster,
    setReaderMode: ctrl.setReaderMode,
    setHideEnded: ctrl.setHideEnded,
    handleAnswerApproval,
    handleLocate,
    focusReader: () => workspace.focusRegion("tasks.reader"),
  });

  const legacySwitch = <LegacyCallsSwitch onSwitch={(): void => { closeBackgroundTasks(); requestDesktopWorkView("conversation"); }} />;
  const showRosterPane = wide || ctrl.compactShowRoster || !selected;
  const showReaderPane = wide || (!ctrl.compactShowRoster && Boolean(selected));

  return (
    <Box ref={rootRef} data-desktop-tasks-view={sessionId} data-desktop-tasks-layout={wide ? "split" : "compact"} onKeyDown={handleKeyDown} sx={{ flex: 1, minHeight: 0, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
      {!wide && background.tasks.length > 0 && (
        <DesktopTasksCompactStrip totalCount={background.tasks.length} visibleTasks={visibleTasks} selectedKey={selected?.key ?? null} showRoster={ctrl.compactShowRoster} onToggleRoster={(): void => ctrl.setCompactShowRoster((open) => !open)} onSelect={ctrl.chooseTask} />
      )}
      <Box sx={{ flex: 1, minHeight: 0, minWidth: 0, display: "flex", overflow: "hidden" }}>
        {showRosterPane && (
          <Box data-desktop-region="tasks.roster" data-desktop-focus-default={!showReaderPane ? "true" : undefined} tabIndex={-1} sx={{ ...(wide ? { width: 236, flexShrink: 0, borderRight: 1, borderColor: "divider" } : { flex: 1 }), minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column", overflowY: "auto", outline: "none", ...desktopScrollbarSx }}>
            <TaskRoster sessionId={sessionId} tasks={background.tasks} calls={background.calls} selectedKey={selected?.key ?? null} desktop hideEnded={ctrl.hideEnded} onToggleHideEnded={(): void => ctrl.setHideEnded((value) => !value)} collapsedKeys={ctrl.collapsedKeys} onToggleFold={ctrl.toggleFold} onSelect={ctrl.chooseTask} onRefreshCalls={background.refreshCalls} now={ctrl.now} />
            <Box sx={{ flex: 1 }} />
            {legacySwitch}
            <ScrollEdges />
          </Box>
        )}
        {showReaderPane && (
          <Box data-desktop-region="tasks.reader" data-desktop-focus-default="true" tabIndex={-1} sx={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden", outline: "none" }}>
            {selected
              ? (
                <TaskResultReader sessionId={sessionId} task={selected} background={background} subagentTranscript={subagentTranscript} desktop modeOverride={currentMode} onModeChange={(mode): void => ctrl.setReaderMode({ key: selected.key, mode })} onAnswerApproval={handleAnswerApproval} onLocate={selected.anchorSeq !== null ? handleLocate : undefined} onOpenOriginTool={handleOpenOriginTool} onQuoted={(): void => void requestAnimationFrame(() => workspace.focusRegion("prompt.composer"))} extraInfoFooter={!wide ? legacySwitch : undefined} />
              )
              : (
                <Box sx={{ p: 3, color: "text.secondary" }}>
                  <Typography variant="body2">
                    {summary.extraActivity ? "The agent reports background activity that is not listed as an individual task yet." : "No background tasks in this session."}
                  </Typography>
                </Box>
              )}
          </Box>
        )}
      </Box>
    </Box>
  );
}
