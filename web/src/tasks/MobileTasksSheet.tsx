// Mobile Background Tasks surface (390x844 iOS PWA):
// 1-tap content-height Triage Peek -> same-host Reader + 1-tap bottom worker switcher.

import { FloatingActionIsland } from "@cowboy/app-shell";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { alpha, Box, ButtonBase, Chip, Stack, Typography } from "@mui/material";
import { ChatBubbleOutline, ChevronLeft, ChevronRight, FormatListBulleted, FormatQuoteOutlined, InfoOutlined, SubjectOutlined, TerminalOutlined } from "@mui/icons-material";
import { acknowledgeTask, taskHash, taskKeyFromHash, type TaskSummary } from "../backgroundTasks";
import { closeBackgroundTasks, inspectorState, openBackgroundTask, OPEN_BACKGROUND_TASKS_EVENT, selectBackgroundTask, useInspectorState } from "../backgroundTaskInspectorState";
import { appendDraftText } from "../draftStore";
import { Sheet } from "../Sheet";
import { notify } from "../store";
import { taskToneColor } from "../SubagentCard";
import { useSubagentTranscript } from "../SubagentTranscript";
import { taskTone } from "../TaskRow";
import type { BackgroundTasks } from "../useBackgroundTasks";
import { LegacyCallsSwitch } from "./TaskDiagnostics";
import { TaskResultReader, type TaskReaderViewMode } from "./TaskResultReader";
import { TaskRoster } from "./TaskRoster";
import { buildTaskForest, flattenTaskSections, formatTaskQuote, isTaskEnded, taskPillLabel, taskTabSummary } from "./taskTree";

type MobileLevel = "peek" | "reader" | "info";

function defaultMobileMode(task: TaskSummary): TaskReaderViewMode {
  return isTaskEnded(task) ? "result" : task.source === "call" ? "transcript" : "result";
}

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

function useMobileTasksHistory({ sessionId, background, visibleTasks }: { sessionId: string; background: BackgroundTasks; visibleTasks: readonly TaskSummary[] }) {
  const state = useInspectorState();
  const open = state.sessionId === sessionId;
  const [level, setLevel] = useState<MobileLevel>("peek");
  const [readerMode, setReaderMode] = useState<TaskReaderViewMode>("result");
  const depth = useRef(0);
  const closingHash = useRef<string | null>(null);
  const selected = state.key ? background.byKey.get(state.key) ?? null : null;

  useEffect(() => {
    const hash = globalThis.location?.hash ?? "";
    const key = taskKeyFromHash(hash);
    if (!key) { closingHash.current = null; return; }
    if (closingHash.current === hash || inspectorState().sessionId === sessionId || !background.byKey.has(key)) return;
    openBackgroundTask(sessionId, key, "link");
  }, [sessionId, background.byKey]);

  useEffect(() => {
    const onOpen = (event: Event): void => {
      const detail = (event as CustomEvent<{ sessionId?: string }>).detail;
      if (detail?.sessionId && detail.sessionId !== sessionId) return;
      openBackgroundTask(sessionId, null, "command");
    };
    globalThis.addEventListener(OPEN_BACKGROUND_TASKS_EVENT, onOpen);
    return () => globalThis.removeEventListener(OPEN_BACKGROUND_TASKS_EVENT, onOpen);
  }, [sessionId]);

  useEffect(() => {
    if (!open) { depth.current = 0; return; }
    const target = state.key ? background.byKey.get(state.key) ?? null : null;
    if (target) {
      setLevel("reader");
      setReaderMode(defaultMobileMode(target));
      if (state.origin !== "link" && target.attention === "failed_unseen") acknowledgeTask(sessionId, target);
    } else {
      setLevel("peek");
    }
    if (depth.current === 0) {
      globalThis.history?.pushState({ cowboyTasks: target ? 2 : 1 }, "", target ? taskHash(target.key) : "#tasks");
      depth.current = 1;
    }
  }, [open, state.revision, state.key, state.origin, sessionId, background.byKey]);

  const close = useCallback((): void => {
    const steps = depth.current;
    depth.current = 0;
    const hash = globalThis.location?.hash ?? "";
    if (taskKeyFromHash(hash)) closingHash.current = hash;
    closeBackgroundTasks();
    if (steps > 0) globalThis.history?.go(-steps);
  }, []);

  useEffect(() => {
    if (!open) return undefined;
    const onPop = (): void => {
      if (depth.current <= 1) { depth.current = 0; closeBackgroundTasks(); return; }
      depth.current -= 1;
      if (level === "info") { setLevel("reader"); setReaderMode("result"); }
      else if (level === "reader") { setLevel("peek"); selectBackgroundTask(null); }
    };
    globalThis.addEventListener("popstate", onPop);
    return () => globalThis.removeEventListener("popstate", onPop);
  }, [open, level]);

  const openWorkerFromPeek = useCallback((key: string): void => {
    const target = background.byKey.get(key);
    if (!target) return;
    selectBackgroundTask(key);
    setLevel("reader");
    setReaderMode(defaultMobileMode(target));
    if (target.attention === "failed_unseen") acknowledgeTask(sessionId, target);
    globalThis.history?.pushState({ cowboyTasks: 2 }, "", taskHash(key));
    depth.current += 1;
  }, [background.byKey, sessionId]);

  const switchSibling = useCallback((key: string): void => {
    const target = background.byKey.get(key);
    if (!target) return;
    selectBackgroundTask(key);
    setLevel("reader");
    setReaderMode(defaultMobileMode(target));
    if (target.attention === "failed_unseen") acknowledgeTask(sessionId, target);
    globalThis.history?.replaceState({ cowboyTasks: 2 }, "", taskHash(key));
  }, [background.byKey, sessionId]);

  const stepSibling = useCallback((delta: -1 | 1): void => {
    if (visibleTasks.length <= 1 || !selected) return;
    const index = visibleTasks.findIndex((item) => item.key === selected.key);
    const next = visibleTasks[(index + delta + visibleTasks.length) % visibleTasks.length];
    if (next) switchSibling(next.key);
  }, [selected, switchSibling, visibleTasks]);

  const openInfo = useCallback((): void => {
    if (!selected) return;
    if (level === "info") { setLevel("reader"); setReaderMode("result"); if (depth.current > 1) globalThis.history?.back(); return; }
    setLevel("info");
    setReaderMode("info");
    globalThis.history?.pushState({ cowboyTasks: 3 }, "", taskHash(selected.key));
    depth.current += 1;
  }, [level, selected]);

  const showPeek = useCallback((): void => {
    setLevel("peek");
    selectBackgroundTask(null);
    globalThis.history?.replaceState({ cowboyTasks: 1 }, "", "#tasks");
  }, []);

  return { open, selected, level, readerMode, setLevel, setReaderMode, close, openWorkerFromPeek, switchSibling, stepSibling, openInfo, showPeek };
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
  const answerApproval = useCallback((): void => {
    nav.close();
    requestAnimationFrame(() => {
      const approval = document.querySelector<HTMLElement>("[data-call-approval], [data-permission-overlay]");
      approval?.scrollIntoView({ block: "nearest" });
      approval?.querySelector<HTMLElement>("button")?.focus({ preventScroll: true });
    });
  }, [nav]);

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
          onToggleLog={(): void => { nav.setLevel("reader"); nav.setReaderMode((current) => (current === "transcript" ? "result" : "transcript")); }}
          onToggleInfo={nav.openInfo}
          onQuote={(): void => { appendDraftText(sessionId, formatTaskQuote(nav.selected!)); notify("Quoted to Prompt", "info"); nav.close(); }}
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
              <TaskRoster sessionId={sessionId} tasks={tasks} calls={calls} selectedKey={null} desktop={false} hideEnded={hideEnded} onToggleHideEnded={(): void => setHideEnded((value) => !value)} collapsedKeys={collapsedKeys} onToggleFold={(key): void => setCollapsedKeys((prev) => { const next = new Set(prev); if (next.has(key)) next.delete(key); else next.add(key); return next; })} onSelect={nav.openWorkerFromPeek} onRefreshCalls={refreshCalls} onReviewRequest={(): void => answerApproval()} />
              <LegacyCallsSwitch onSwitch={nav.close} />
            </Stack>
          )
          : (
            <TaskResultReader sessionId={sessionId} task={nav.selected!} background={background} subagentTranscript={subagentTranscript} desktop={false} modeOverride={effectiveMode} onModeChange={(next): void => { if (next === "info") nav.openInfo(); else { nav.setLevel("reader"); nav.setReaderMode(next); } }} onAnswerApproval={answerApproval} onOpenOriginTool={onOpenTool ? (toolKey): void => { nav.close(); onOpenTool(toolKey); } : undefined} onQuoted={nav.close} bottomInset="var(--cowboy-sheet-dock-clearance, 108px)" />
          )}
      </Box>
    </Sheet>
  );
}
