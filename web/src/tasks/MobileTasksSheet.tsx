// Mobile Background Tasks surface (390x844 iOS PWA):
// 1-tap content-height Triage Peek -> same-host Reader + 1-tap bottom worker switcher.

import { FloatingActionIsland } from "@cowboy/app-shell";
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { alpha, Box, ButtonBase, Chip, Stack, Typography } from "@mui/material";
import { ChatBubbleOutline, ChevronLeft, ChevronRight, FormatListBulleted, FormatQuoteOutlined, InfoOutlined, SubjectOutlined, TerminalOutlined } from "@mui/icons-material";
import type { TaskSummary } from "../backgroundTasks";
import { Sheet } from "../Sheet";
import { taskToneColor } from "../SubagentCard";
import { useSubagentTranscript } from "../SubagentTranscript";
import { taskTone } from "../TaskRow";
import type { BackgroundTasks } from "../useBackgroundTasks";
import { LegacyCallsSwitch } from "./TaskDiagnostics";
import { focusTaskApprovalSurface, quoteTaskToPrompt } from "./taskEvents";
import { type TaskReaderViewMode, TaskResultReader } from "./TaskResultReader";
import { TaskRoster } from "./TaskRoster";
import { buildTaskForest, flattenTaskSections, taskPillLabel, taskTabSummary } from "./taskTree";
import { useMobileTasksHistory } from "./useMobileTasksHistory";

function DockActionButton({ label, ariaLabel, icon, active = false, dataAttr, onClick }: { label: string; ariaLabel?: string; icon: React.ReactNode; active?: boolean; dataAttr?: string; onClick: () => void }): React.JSX.Element {
  return (
    <ButtonBase aria-label={ariaLabel ?? label} aria-pressed={active || undefined} {...(dataAttr ? { [dataAttr]: "true" } : {})} onClick={(event): void => { event.stopPropagation(); onClick(); }} onPointerDown={(event): void => event.stopPropagation()} sx={(theme) => ({ minWidth: 52, height: 44, px: 1, borderRadius: 999, display: "inline-flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 0.15, color: active ? "primary.main" : "text.primary", bgcolor: active ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.2 : 0.12) : "transparent", "& .MuiSvgIcon-root": { fontSize: "1.125rem", width: "1.125rem", height: "1.125rem" }, "&:active": { bgcolor: "action.selected" } })}>
      {icon}
      <Typography component="span" sx={{ fontSize: "0.625rem", fontWeight: 650, lineHeight: 1, whiteSpace: "nowrap" }}>{label}</Typography>
    </ButtonBase>
  );
}

function MobileWorkerSwitcherStrip({ tasks, selectedKey, onSelectSibling, onStepSibling }: { tasks: readonly TaskSummary[]; selectedKey: string; onSelectSibling: (key: string) => void; onStepSibling: (delta: -1 | 1) => void }): React.JSX.Element {
  return (
    <Box data-mobile-worker-strip sx={(theme) => ({ pointerEvents: "auto", display: "flex", alignItems: "center", gap: 0.5, px: 0.5, py: 0.4, borderRadius: 999, border: 1, borderColor: alpha(theme.palette.common.white, theme.palette.mode === "dark" ? 0.16 : 0.55), bgcolor: alpha(theme.palette.background.paper, theme.palette.mode === "dark" ? 0.72 : 0.76), backdropFilter: "blur(24px) saturate(180%)", WebkitBackdropFilter: "blur(24px) saturate(180%)", boxShadow: `0 6px 20px ${alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.34 : 0.12)}` })}>
      {tasks.length > 3 && (
        <ButtonBase aria-label="Previous worker" data-mobile-worker-prev onClick={(): void => onStepSibling(-1)} sx={{ width: 40, height: 40, borderRadius: 999, flexShrink: 0, color: "text.secondary" }}>
          <ChevronLeft fontSize="small" />
        </ButtonBase>
      )}
      <Box sx={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 0.5, overflowX: "auto", scrollbarWidth: "none", "&::-webkit-scrollbar": { display: "none" } }}>
        {tasks.map((task) => {
          const active = task.key === selectedKey;
          const tone = taskTone(task.group);
          return (
            <ButtonBase key={task.key} data-mobile-worker-pill={task.key} aria-current={active ? "true" : undefined} aria-label={`${task.title}: ${task.chip}`} onClick={(): void => onSelectSibling(task.key)} sx={(theme) => ({ flexShrink: 0, maxWidth: 136, height: 40, px: 1.25, gap: 0.6, borderRadius: 999, fontSize: "0.75rem", fontWeight: active ? 700 : 550, bgcolor: active ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.24 : 0.15) : "transparent", color: active ? "text.primary" : "text.secondary", border: 1, borderColor: active ? "primary.main" : "transparent" })}>
              <Box component="span" sx={(theme) => ({ width: 6, height: 6, borderRadius: "50%", flexShrink: 0, bgcolor: taskToneColor(tone, theme) })} />
              <Typography variant="caption" noWrap sx={{ fontWeight: "inherit", fontSize: "inherit" }}>{taskPillLabel(task)}</Typography>
            </ButtonBase>
          );
        })}
      </Box>
      {tasks.length > 3 && (
        <ButtonBase aria-label="Next worker" data-mobile-worker-next onClick={(): void => onStepSibling(1)} sx={{ width: 40, height: 40, borderRadius: 999, flexShrink: 0, color: "text.secondary" }}>
          <ChevronRight fontSize="small" />
        </ButtonBase>
      )}
    </Box>
  );
}

function MobileReaderDock({ tasks, selected, mode, hasConversation, onClose, onShowPeek, onSelectSibling, onStepSibling, onToggleLog, onToggleInfo, onQuote }: { tasks: readonly TaskSummary[]; selected: TaskSummary; mode: TaskReaderViewMode; hasConversation: boolean; onClose: () => void; onShowPeek: () => void; onSelectSibling: (key: string) => void; onStepSibling: (delta: -1 | 1) => void; onToggleLog: () => void; onToggleInfo: () => void; onQuote: () => void }): React.JSX.Element {
  const dockRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const footer = dockRef.current?.closest<HTMLElement>("[data-detent-sheet-footer]");
    const dialog = dockRef.current?.closest<HTMLElement>('[role="dialog"]');
    if (!footer || !dialog) return undefined;
    dialog.style.setProperty("--cowboy-sheet-footer-inline", "12px");
    const measure = (): void => dialog.style.setProperty("--cowboy-sheet-dock-clearance", `${String(Math.ceil(footer.getBoundingClientRect().height))}px`);
    const observer = new ResizeObserver(measure);
    observer.observe(footer);
    measure();
    return () => {
      observer.disconnect();
      dialog.style.removeProperty("--cowboy-sheet-dock-clearance");
      dialog.style.removeProperty("--cowboy-sheet-footer-inline");
    };
  }, []);
  return (
    <Box ref={dockRef} data-mobile-sheet-footer-shield data-mobile-tasks-dock sx={{ width: "100%", display: "flex", flexDirection: "column", gap: 0.65, pointerEvents: "none" }}>
      {tasks.length > 1 && <MobileWorkerSwitcherStrip tasks={tasks} selectedKey={selected.key} onSelectSibling={onSelectSibling} onStepSibling={onStepSibling} />}
      <Box sx={{ display: "flex", justifyContent: "center", width: "100%", pointerEvents: "none" }}>
        <FloatingActionIsland maxWidth="100%" minHeight={52}>
          <Stack direction="row" alignItems="center" justifyContent="space-evenly" spacing={0.25}>
            <DockActionButton label="Chat" ariaLabel="Close" dataAttr="data-mobile-tasks-chat" icon={<ChatBubbleOutline />} onClick={onClose} />
            <DockActionButton label="All" dataAttr="data-mobile-tasks-all" icon={<FormatListBulleted />} onClick={onShowPeek} />
            {hasConversation && <DockActionButton label={mode === "transcript" ? "Result" : "Log"} dataAttr="data-mobile-tasks-toggle-log" active={mode === "transcript"} icon={mode === "transcript" ? <SubjectOutlined /> : <TerminalOutlined />} onClick={onToggleLog} />}
            <DockActionButton label="Info" dataAttr="data-mobile-tasks-info" active={mode === "info"} icon={<InfoOutlined />} onClick={onToggleInfo} />
            <DockActionButton label="Quote" dataAttr="data-mobile-tasks-quote" icon={<FormatQuoteOutlined />} onClick={onQuote} />
          </Stack>
        </FloatingActionIsland>
      </Box>
    </Box>
  );
}

export function MobileTasksSheet({ sessionId, background, onOpenTool }: { sessionId: string; background: BackgroundTasks; onOpenTool?: ((toolKey: string) => void) | undefined }): React.JSX.Element | null {
  const { tasks, calls, refreshCalls, backgroundActivity } = background;
  const [hideEnded, setHideEnded] = useState(false);
  const [collapsedKeys, setCollapsedKeys] = useState<ReadonlySet<string>>(() => new Set());
  const forest = useMemo(() => buildTaskForest(tasks), [tasks]);
  const visibleTasks = useMemo(() => flattenTaskSections(forest, { collapsedKeys, hideEnded }).map((row) => row.task), [forest, collapsedKeys, hideEnded]);
  const summary = useMemo(() => taskTabSummary(tasks, backgroundActivity), [tasks, backgroundActivity]);
  const nav = useMobileTasksHistory({ sessionId, background, visibleTasks });
  const subagentTranscript = useSubagentTranscript(sessionId, nav.open && nav.selected?.source === "subagent" ? (nav.selected.subagent ?? null) : null);
  const hasConversation = Boolean(nav.selected && (nav.selected.source === "call" || (nav.selected.source === "subagent" && subagentTranscript.observed)));

  const answerApproval = useCallback((task: TaskSummary): void => {
    nav.close();
    requestAnimationFrame(() => focusTaskApprovalSurface(sessionId, task));
  }, [nav, sessionId]);

  if (!nav.open) return null;
  const isPeek = nav.level === "peek" || !nav.selected;
  const effectiveMode: TaskReaderViewMode = nav.level === "info" ? "info" : nav.readerMode;
  const statusPills = [
    summary.needsYouCount > 0 ? `${String(summary.needsYouCount)} needs you` : null,
    summary.activeCount > 0 ? `${String(summary.activeCount)} active` : null,
    summary.failedUnseenCount > 0 ? `${String(summary.failedUnseenCount)} failed` : null,
  ].filter((value): value is string => value !== null);
  if (statusPills.length === 0 && summary.endedCount > 0) statusPills.push(`${String(summary.endedCount)} ended`);

  const peekFooter = (
    <Stack direction="row" alignItems="center" justifyContent="space-between" data-mobile-tasks-peek-footer sx={{ width: "100%", px: 0.5 }}>
      <ButtonBase aria-label="Close" data-mobile-tasks-chat onClick={nav.close} sx={(theme) => ({ height: 40, px: 1.75, gap: 0.75, borderRadius: 999, border: 1, borderColor: "divider", bgcolor: alpha(theme.palette.background.paper, 0.85), fontWeight: 650, fontSize: "0.8125rem" })}>
        <ChatBubbleOutline fontSize="small" />
        Chat
      </ButtonBase>
      <Typography variant="caption" color="text.secondary" sx={{ fontWeight: 600 }}>
        {statusPills.join(" · ") || `${String(tasks.length)} tasks`}
      </Typography>
    </Stack>
  );

  return (
    <Sheet
      open
      onClose={nav.close}
      title={isPeek ? "Background tasks" : nav.selected!.title}
      hideMobileTitle={!isPeek}
      detent
      forceSheet
      portal
      cover={!isPeek}
      maxHeight={isPeek ? "min(52dvh, 440px)" : undefined}
      peekDetent={false}
      fill={!isPeek}
      readerTopBleed={!isPeek}
      floatingActions={!isPeek}
      footerDock={isPeek ? peekFooter : (
        <MobileReaderDock
          tasks={visibleTasks.length > 0 ? visibleTasks : tasks}
          selected={nav.selected!}
          mode={effectiveMode}
          hasConversation={hasConversation}
          onClose={nav.close}
          onShowPeek={nav.showPeek}
          onSelectSibling={nav.switchSibling}
          onStepSibling={nav.stepSibling}
          onToggleLog={(): void => nav.setContentMode(effectiveMode === "transcript" ? "result" : "transcript")}
          onToggleInfo={nav.toggleInfo}
          onQuote={(): void => quoteTaskToPrompt(sessionId, nav.selected!, nav.close)}
        />
      )}
    >
      <Box data-background-tasks-inspector={sessionId} data-mobile-task-level={nav.level} sx={{ color: "text.primary", typography: "body2", ...(isPeek ? { pb: 1 } : { flex: 1, minHeight: 0, minWidth: 0, display: "flex", flexDirection: "column", pt: "var(--cowboy-reader-top-inset, 0px)" }) }}>
        {isPeek
          ? (
            <Stack spacing={1} data-mobile-triage-peek>
              {statusPills.length > 0 && (
                <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
                  {statusPills.map((pill) => <Chip key={pill} size="small" variant="outlined" label={pill} sx={{ height: 22, fontSize: "0.7rem" }} />)}
                </Stack>
              )}
              <TaskRoster sessionId={sessionId} tasks={tasks} calls={calls} selectedKey={null} desktop={false} hideEnded={hideEnded} onToggleHideEnded={(): void => setHideEnded((value) => !value)} collapsedKeys={collapsedKeys} onToggleFold={(key): void => setCollapsedKeys((prev) => { const next = new Set(prev); if (next.has(key)) next.delete(key); else next.add(key); return next; })} onSelect={nav.openWorkerFromPeek} onRefreshCalls={refreshCalls} onReviewRequest={answerApproval} />
              <LegacyCallsSwitch onSwitch={nav.close} />
            </Stack>
          )
          : (
            <TaskResultReader sessionId={sessionId} task={nav.selected!} background={background} subagentTranscript={subagentTranscript} desktop={false} mode={effectiveMode} onModeChange={(next): void => { if (next === "info") nav.toggleInfo(); else nav.setContentMode(next); }} onAnswerApproval={(): void => answerApproval(nav.selected!)} onOpenOriginTool={onOpenTool ? (toolKey): void => { nav.close(); onOpenTool(toolKey); } : undefined} onOpenChildSession={nav.openChildSession} onQuoted={nav.close} bottomInset="var(--cowboy-sheet-dock-clearance, 108px)" />
          )}
      </Box>
    </Sheet>
  );
}
