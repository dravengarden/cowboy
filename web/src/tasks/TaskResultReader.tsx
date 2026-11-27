// Result-first task reader shared by Desktop Work Pane Tasks and Mobile Reader.

import { ScrollEdges } from "@cowboy/app-shell";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { alpha, Box, Button, Chip, CircularProgress, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import { FormatQuoteOutlined, InfoOutlined, MyLocation, OpenInNew, StopCircleOutlined, SubjectOutlined, TerminalOutlined } from "@mui/icons-material";
import { acknowledgeTask, originToolKey, type TaskSummary } from "../backgroundTasks";
import { setActiveSessionId } from "../controlPlane";
import { desktopScrollbarSx } from "../desktop/desktopScrollbar";
import { callConversationId, callModelLabel, type ManagedCallDetail, useManagedCall } from "../managedCalls";
import { CallConversation, CallDetailView, ResultSection } from "../ManagedCallsDock";
import { SubagentDetail, SubagentStopConfirm } from "../SubagentCard";
import { SubagentConversation, type SubagentTranscriptState } from "../SubagentTranscript";
import { TaskChip } from "../TaskRow";
import type { BackgroundTasks } from "../useBackgroundTasks";
import { TaskDiagnostics } from "./TaskDiagnostics";
import { quoteTaskToPrompt, rememberCallDetail, rememberTaskReaderScroll, savedTaskReaderScroll } from "./taskEvents";
import { callVerdictLabel, isTaskEnded, taskTimeLabel } from "./taskTree";
import { useBoundStopConfirm } from "./useBoundStopConfirm";

export type TaskReaderViewMode = "result" | "transcript" | "info";
export type TaskReaderContentMode = "result" | "transcript";

export function defaultReaderMode(task: TaskSummary, observedSubagent: boolean): TaskReaderContentMode {
  if (task.source === "approval" || isTaskEnded(task)) return "result";
  if (task.source === "call") return "transcript";
  return observedSubagent ? "transcript" : "result";
}

function CallResultSummaryBody({
  task,
  desktop,
  detail,
  error,
}: {
  task: TaskSummary;
  desktop: boolean;
  detail: ManagedCallDetail | null;
  error: string | null;
}): React.JSX.Element {
  const call = task.call!;
  const verdict = callVerdictLabel(call);
  return (
    <Stack spacing={1.25} data-task-result-body={task.key}>
      {call.error && (
        <Box sx={(theme) => ({ p: 1.25, borderRadius: 1.5, border: 1, borderColor: alpha(theme.palette.error.main, 0.35), bgcolor: alpha(theme.palette.error.main, theme.palette.mode === "dark" ? 0.14 : 0.06) })}>
          <Typography variant="caption" color="error" sx={{ fontWeight: 700, display: "block" }}>
            Failure: {call.error.code.replaceAll("_", " ")}
          </Typography>
          {call.error.detail && (
            <Typography variant="body2" sx={{ mt: 0.25, whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
              {call.error.detail}
            </Typography>
          )}
        </Box>
      )}
      {verdict && (
        <Stack direction="row" spacing={0.75} alignItems="center" flexWrap="wrap">
          <Chip size="small" data-task-verdict-chip color={call.verdict === "approve" ? "success" : "warning"} variant="outlined" label={verdict} />
          <Typography variant="caption" color="text.secondary">{callModelLabel(call)}</Typography>
        </Stack>
      )}
      {error && !detail && <Typography variant="caption" color="error">{error}</Typography>}
      {detail
        ? <ResultSection detail={detail} touch={!desktop} />
        : !error && (
          <Stack direction="row" spacing={1} alignItems="center" sx={{ py: 1 }}>
            <CircularProgress size="0.95rem" />
            <Typography variant="caption" color="text.secondary">Loading result…</Typography>
          </Stack>
        )}
    </Stack>
  );
}

function TaskReaderHeader({
  sessionId,
  task,
  desktop,
  activeMode,
  hasConversation,
  stopStopping,
  stopMessage,
  onToggleInfo,
  onToggleTranscript,
  onQuote,
  onOpenChild,
  onLocate,
  onStopClick,
}: {
  sessionId: string;
  task: TaskSummary;
  desktop: boolean;
  activeMode: TaskReaderViewMode;
  hasConversation: boolean;
  stopStopping: boolean;
  stopMessage: string | null;
  onToggleInfo: () => void;
  onToggleTranscript: () => void;
  onQuote: () => void;
  onOpenChild: () => void;
  onLocate?: (() => void) | undefined;
  onStopClick: () => void;
}): React.JSX.Element {
  const elapsed = taskTimeLabel(task);
  const verdict = task.call ? callVerdictLabel(task.call) : null;
  const modelMeta = task.call ? callModelLabel(task.call) : task.subagent?.model ?? null;
  return (
    <Box data-task-reader-header sx={(theme) => ({ px: desktop ? 1.5 : 1.75, py: desktop ? 0.85 : 1, borderBottom: 1, borderColor: "divider", flexShrink: 0, display: "flex", alignItems: "center", gap: 1, minWidth: 0, ...(!desktop ? { bgcolor: alpha(theme.palette.background.paper, theme.palette.mode === "dark" ? 0.8 : 0.86), backdropFilter: "blur(20px) saturate(180%)", WebkitBackdropFilter: "blur(20px) saturate(180%)", zIndex: 1 } : {}) })}>
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
          <Typography variant={desktop ? "subtitle2" : "subtitle1"} noWrap sx={{ fontWeight: 700, minWidth: 0 }}>{task.title}</Typography>
          <TaskChip task={task} />
          {elapsed && <Typography variant="caption" color="text.secondary" sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}>{elapsed}</Typography>}
        </Stack>
        {(modelMeta || verdict || task.health || stopMessage) && (
          <Typography variant="caption" noWrap sx={{ display: "block", color: stopMessage ? "warning.main" : "text.secondary" }}>
            {[verdict, modelMeta, task.health, stopMessage].filter(Boolean).join(" · ")}
          </Typography>
        )}
      </Box>
      <Stack direction="row" spacing={0.5} alignItems="center" sx={{ flexShrink: 0 }}>
        {task.attention === "failed_unseen" && (
          <Button size="small" variant="outlined" color="error" data-task-mark-seen onClick={(): void => acknowledgeTask(sessionId, task)} sx={{ textTransform: "none", minHeight: desktop ? 26 : 32 }}>
            Mark as seen
          </Button>
        )}
        {task.canStop && (
          <Button size="small" variant="outlined" color="warning" disabled={stopStopping} data-task-detail-stop startIcon={<StopCircleOutlined fontSize="small" />} onClick={onStopClick} sx={{ textTransform: "none", minHeight: desktop ? 26 : 32 }}>
            {stopStopping ? "Stopping…" : "Stop"}
          </Button>
        )}
        {desktop && (
          <>
            {hasConversation && (
              <Tooltip title={activeMode === "transcript" ? "Show Result summary (T)" : "Show Transcript (T)"}>
                <Button size="small" variant={activeMode === "transcript" ? "contained" : "outlined"} data-task-mode-transcript startIcon={activeMode === "transcript" ? <SubjectOutlined fontSize="small" /> : <TerminalOutlined fontSize="small" />} onClick={onToggleTranscript} sx={{ textTransform: "none", minHeight: 26 }}>
                  {activeMode === "transcript" ? "Result" : "Transcript"}
                </Button>
              </Tooltip>
            )}
            <Tooltip title="Quote to Prompt (S)">
              <IconButton size="small" aria-label="Quote to Prompt" data-task-quote-prompt onClick={onQuote}><FormatQuoteOutlined fontSize="small" /></IconButton>
            </Tooltip>
            {task.source === "call" && task.call && (
              <Tooltip title="Open as session (O)">
                <IconButton size="small" aria-label="Open as session" data-task-open-session onClick={onOpenChild}><OpenInNew fontSize="small" /></IconButton>
              </Tooltip>
            )}
            {onLocate && (
              <Tooltip title="Locate in Conversation (Shift+L)">
                <IconButton size="small" aria-label="Locate in Conversation" data-task-locate onClick={onLocate}><MyLocation fontSize="small" /></IconButton>
              </Tooltip>
            )}
            <Tooltip title="Task info & diagnostics (I)">
              <IconButton size="small" aria-label="Task info" aria-pressed={activeMode === "info"} data-task-info-toggle color={activeMode === "info" ? "primary" : "default"} onClick={onToggleInfo}><InfoOutlined fontSize="small" /></IconButton>
            </Tooltip>
          </>
        )}
      </Stack>
    </Box>
  );
}

function TaskReaderSummaryBody({
  task,
  desktop,
  hasConversation,
  originKey,
  callDetail,
  callError,
  onAnswerApproval,
  onOpenTranscript,
  onQuote,
  onOpenOriginTool,
}: {
  task: TaskSummary;
  desktop: boolean;
  hasConversation: boolean;
  originKey: string | null;
  callDetail: ManagedCallDetail | null;
  callError: string | null;
  onAnswerApproval: () => void;
  onOpenTranscript: () => void;
  onQuote: () => void;
  onOpenOriginTool?: ((toolKey: string) => void) | undefined;
}): React.JSX.Element {
  return (
    <Stack spacing={1.5} data-task-detail={task.key}>
      {(task.source === "approval" || task.group === "needs_you") && (
        <Box sx={(theme) => ({ p: 1.5, borderRadius: 2, border: 1, borderColor: alpha(theme.palette.warning.main, 0.45), bgcolor: alpha(theme.palette.warning.main, theme.palette.mode === "dark" ? 0.14 : 0.08) })}>
          <Typography variant="subtitle2" color="warning.main" sx={{ fontWeight: 700 }}>
            {task.source === "approval" ? "Call approval needed" : `${String(task.pendingRequests)} pending request${task.pendingRequests === 1 ? "" : "s"}`}
          </Typography>
          {task.subtitle && <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>{task.subtitle}</Typography>}
          <Button size="small" variant="contained" color="warning" disableElevation data-task-answer-approval onClick={onAnswerApproval} sx={{ mt: 1.25, textTransform: "none", fontWeight: 650 }}>
            Review request
          </Button>
        </Box>
      )}
      {task.source === "call" && task.call && (
        <CallResultSummaryBody task={task} desktop={desktop} detail={callDetail} error={callError} />
      )}
      {task.source === "subagent" && task.subagent && (
        <SubagentDetail subagent={task.subagent} resumed={task.subagent.revision > 1 && task.subagent.terminalSeq !== null} />
      )}
      <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap sx={{ pt: 0.5 }}>
        {hasConversation && (
          <Button size="small" variant="outlined" data-task-open-transcript startIcon={<TerminalOutlined fontSize="small" />} onClick={onOpenTranscript} sx={{ textTransform: "none" }}>
            {isTaskEnded(task) ? "Full transcript" : "Live transcript"}
          </Button>
        )}
        <Button size="small" variant="text" data-task-quote-inline startIcon={<FormatQuoteOutlined fontSize="small" />} onClick={onQuote} sx={{ textTransform: "none" }}>
          Quote to Prompt
        </Button>
        {originKey && onOpenOriginTool && (
          <Button size="small" variant="text" data-task-open-origin onClick={(): void => onOpenOriginTool(originKey)} sx={{ textTransform: "none" }}>
            Open originating tool card
          </Button>
        )}
      </Stack>
    </Stack>
  );
}

function TaskReaderInfoPanel({
  sessionId,
  task,
  desktop,
  originKey,
  callQuery,
  onOpenChild,
  onOpenOriginTool,
  extraInfoFooter,
}: {
  sessionId: string;
  task: TaskSummary;
  desktop: boolean;
  originKey: string | null;
  callQuery: { detail: ManagedCallDetail | null; error: string | null };
  onOpenChild: () => void;
  onOpenOriginTool?: ((toolKey: string) => void) | undefined;
  extraInfoFooter?: React.ReactNode;
}): React.JSX.Element {
  return (
    <Box data-task-detail={task.key} data-task-info-panel sx={{ minWidth: 0 }}>
      {task.source === "call" && task.call
        ? <CallDetailView parent={sessionId} call={task.call} desktop={desktop} onOpenChild={onOpenChild} prefetched={callQuery} hideStop />
        : task.source === "subagent" && task.subagent
        ? <SubagentDetail subagent={task.subagent} resumed={task.subagent.revision > 1 && task.subagent.terminalSeq !== null} />
        : <Typography variant="body2" color="text.secondary">{task.subtitle || "Pending call approval request."}</Typography>}
      {originKey && onOpenOriginTool && (
        <Button size="small" variant="text" data-task-open-origin onClick={(): void => onOpenOriginTool(originKey)} sx={{ mt: 1, textTransform: "none" }}>
          Open originating tool card
        </Button>
      )}
      <TaskDiagnostics task={task} />
      {extraInfoFooter}
    </Box>
  );
}

export interface TaskResultReaderProps {
  sessionId: string;
  task: TaskSummary;
  background: BackgroundTasks;
  subagentTranscript: SubagentTranscriptState;
  desktop: boolean;
  mode?: TaskReaderViewMode | undefined;
  onModeChange?: ((mode: TaskReaderViewMode) => void) | undefined;
  onAnswerApproval: () => void;
  onLocate?: (() => void) | undefined;
  onOpenOriginTool?: ((toolKey: string) => void) | undefined;
  onQuoted?: (() => void) | undefined;
  onQuoteRequest?: (() => void) | undefined;
  onOpenChildSession?: ((childSessionId: string) => void) | undefined;
  stopTrigger?: { key: string; nonce: number } | null | undefined;
  onStopTriggerConsumed?: (() => void) | undefined;
  extraInfoFooter?: React.ReactNode;
  bottomInset?: string | undefined;
}

export function TaskResultReader({ sessionId, task, background, subagentTranscript, desktop, mode, onModeChange, onAnswerApproval, onLocate, onOpenOriginTool, onQuoted, onQuoteRequest, onOpenChildSession, stopTrigger, onStopTriggerConsumed, extraInfoFooter, bottomInset }: TaskResultReaderProps): React.JSX.Element {
  const hasConversation = task.source === "call" || (task.source === "subagent" && subagentTranscript.observed);
  const controlled = mode !== undefined;
  const [internalMode, setInternalMode] = useState<{ key: string; mode: TaskReaderViewMode }>(() => ({ key: task.key, mode: defaultReaderMode(task, subagentTranscript.observed) }));
  const activeMode: TaskReaderViewMode = controlled
    ? mode
    : internalMode.key === task.key ? internalMode.mode : defaultReaderMode(task, subagentTranscript.observed);

  const lastContentModeRef = useRef<{ key: string; mode: TaskReaderContentMode }>({ key: task.key, mode: defaultReaderMode(task, subagentTranscript.observed) });
  if (activeMode === "result" || activeMode === "transcript") lastContentModeRef.current = { key: task.key, mode: activeMode };
  const setMode = (next: TaskReaderViewMode): void => {
    if (!controlled) setInternalMode({ key: task.key, mode: next });
    onModeChange?.(next);
  };

  const callId = task.source === "call" && task.call ? task.call.call_id : null;
  const callRev = task.source === "call" && task.call ? task.call.updated_at_ms : 0;
  const callQuery = useManagedCall(sessionId, callId, callRev);
  const callReady = callId === null || (callQuery.detail !== null && callQuery.detail.call_id === callId) || callQuery.error !== null;
  useEffect(() => {
    if (callQuery.detail && callId && callQuery.detail.call_id === callId) rememberCallDetail(sessionId, callQuery.detail);
  }, [sessionId, callId, callQuery.detail]);

  // Keep transcript mounted across Info toggles without a self-triggering effect loop.
  const [mountedTranscript, setMountedTranscript] = useState<{ sessionId: string; key: string } | null>(null);
  useEffect(() => {
    if (activeMode === "transcript" && hasConversation) {
      setMountedTranscript((prev) => prev && prev.sessionId === sessionId && prev.key === task.key ? prev : { sessionId, key: task.key });
    } else {
      setMountedTranscript((prev) => prev && (prev.sessionId !== sessionId || prev.key !== task.key) ? null : prev);
    }
  }, [activeMode, hasConversation, sessionId, task.key]);
  const keepTranscriptMounted = hasConversation && (
    activeMode === "transcript" || (mountedTranscript?.sessionId === sessionId && mountedTranscript.key === task.key)
  );

  const stopGuard = useBoundStopConfirm(sessionId, task);
  const handledStopNonce = useRef(0);
  useEffect(() => {
    if (!stopTrigger || stopTrigger.key !== task.key || stopTrigger.nonce === handledStopNonce.current) return;
    handledStopNonce.current = stopTrigger.nonce;
    stopGuard.trigger();
    onStopTriggerConsumed?.();
  }, [stopTrigger, task.key, stopGuard, onStopTriggerConsumed]);

  const resultScrollRef = useRef<HTMLDivElement>(null);
  const infoScrollRef = useRef<HTMLDivElement>(null);
  const restoredPaneRef = useRef<string | null>(null);
  useLayoutEffect(() => {
    const node = activeMode === "result" ? resultScrollRef.current : activeMode === "info" ? infoScrollRef.current : null;
    if (!node || !callReady) { restoredPaneRef.current = null; return; }
    const paneKey = `${sessionId}\u0000${task.key}\u0000${activeMode}`;
    if (restoredPaneRef.current === paneKey) return;
    node.scrollTop = savedTaskReaderScroll(sessionId, `${task.key}:${activeMode}`);
    restoredPaneRef.current = paneKey;
  }, [sessionId, task.key, activeMode, callReady]);

  const recordScroll = (pane: "result" | "info", top: number): void => {
    if (!callReady || restoredPaneRef.current !== `${sessionId}\u0000${task.key}\u0000${pane}`) return;
    rememberTaskReaderScroll(sessionId, `${task.key}:${pane}`, top);
  };

  const originKey = task.call ? originToolKey(background.items, task.call.request_id) : null;
  const quoteToPrompt = (): void => {
    if (onQuoteRequest) onQuoteRequest();
    else quoteTaskToPrompt(sessionId, task, onQuoted);
  };
  const openChild = (): void => {
    if (!task.call) return;
    const childId = callConversationId(task.call);
    if (onOpenChildSession) onOpenChildSession(childId);
    else setActiveSessionId(childId);
  };
  const toggleInfo = (): void => {
    if (activeMode === "info") {
      setMode(lastContentModeRef.current.key === task.key ? lastContentModeRef.current.mode : defaultReaderMode(task, subagentTranscript.observed));
    } else {
      setMode("info");
    }
  };
  const paneInset = bottomInset ?? (desktop ? 2 : "var(--cowboy-sheet-dock-clearance, 88px)");

  return (
    <Box data-task-reader={task.key} data-task-reader-mode={activeMode} sx={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
      <TaskReaderHeader sessionId={sessionId} task={task} desktop={desktop} activeMode={activeMode} hasConversation={hasConversation} stopStopping={stopGuard.stopStopping} stopMessage={stopGuard.stopMessage} onToggleInfo={toggleInfo} onToggleTranscript={(): void => setMode(activeMode === "transcript" ? "result" : "transcript")} onQuote={quoteToPrompt} onOpenChild={openChild} onLocate={onLocate} onStopClick={stopGuard.trigger} />
      {keepTranscriptMounted && (
        <Box data-task-transcript-host hidden={activeMode !== "transcript"} sx={{ flex: 1, minHeight: 0, minWidth: 0, display: activeMode === "transcript" ? "flex" : "none", position: "relative" }}>
          {task.source === "call" && task.call
            ? <CallConversation call={task.call} desktop={desktop} onDetails={(): void => setMode("info")} />
            : task.source === "subagent" && task.subagent && <SubagentConversation sessionId={sessionId} subagent={task.subagent} state={subagentTranscript} desktop={desktop} />}
        </Box>
      )}
      {activeMode === "result" && (
        <Box ref={resultScrollRef} data-task-reader-scroll="result" onScroll={(event): void => recordScroll("result", event.currentTarget.scrollTop)} sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 2, py: 1.5, pb: paneInset, ...desktopScrollbarSx }}>
          <TaskReaderSummaryBody task={task} desktop={desktop} hasConversation={hasConversation} originKey={originKey} callDetail={callQuery.detail} callError={callQuery.error} onAnswerApproval={onAnswerApproval} onOpenTranscript={(): void => setMode("transcript")} onQuote={quoteToPrompt} onOpenOriginTool={onOpenOriginTool} />
          <ScrollEdges />
        </Box>
      )}
      {activeMode === "info" && (
        <Box ref={infoScrollRef} data-task-reader-scroll="info" onScroll={(event): void => recordScroll("info", event.currentTarget.scrollTop)} sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 2, py: 1.5, pb: paneInset, ...desktopScrollbarSx }}>
          <TaskReaderInfoPanel sessionId={sessionId} task={task} desktop={desktop} originKey={originKey} callQuery={callQuery} onOpenChild={openChild} onOpenOriginTool={onOpenOriginTool} extraInfoFooter={extraInfoFooter} />
          <ScrollEdges />
        </Box>
      )}
      <SubagentStopConfirm open={stopGuard.open} title={stopGuard.pending?.task.title ?? task.title} noun={(stopGuard.pending?.task.source ?? task.source) === "call" ? "call" : "subagent"} onClose={stopGuard.close} onConfirm={stopGuard.confirm} />
    </Box>
  );
}
