// Result-first task reader shared by Desktop Work Pane Tasks and Mobile Reader.

import { ScrollEdges } from "@cowboy/app-shell";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { alpha, Box, Button, Chip, CircularProgress, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import { FormatQuoteOutlined, InfoOutlined, MyLocation, OpenInNew, StopCircleOutlined, SubjectOutlined, TerminalOutlined } from "@mui/icons-material";
import { acknowledgeTask, originToolKey, type TaskSummary } from "../backgroundTasks";
import { setActiveSessionId } from "../controlPlane";
import { appendDraftText } from "../draftStore";
import { desktopScrollbarSx } from "../desktop/desktopScrollbar";
import { callConversationId, callModelLabel, type ManagedCallDetail, useManagedCall } from "../managedCalls";
import { CallConversation, CallDetailView, ResultSection } from "../ManagedCallsDock";
import { SubagentDetail, SubagentStopConfirm } from "../SubagentCard";
import { SubagentConversation, type SubagentTranscriptState } from "../SubagentTranscript";
import { TaskChip } from "../TaskRow";
import { notify } from "../store";
import { type BackgroundTasks, requestTaskStop, STOP_MESSAGE, useStopStates } from "../useBackgroundTasks";
import { TaskDiagnostics } from "./TaskDiagnostics";
import { rememberTaskReaderScroll, savedTaskReaderScroll } from "./taskEvents";
import { callVerdictLabel, formatTaskQuote, isTaskEnded, taskTimeLabel } from "./taskTree";

export type TaskReaderViewMode = "result" | "transcript" | "info";

function defaultReaderMode(task: TaskSummary, observedSubagent: boolean): TaskReaderViewMode {
  if (task.source === "approval" || isTaskEnded(task)) return "result";
  if (task.source === "call") return "transcript";
  return observedSubagent ? "transcript" : "result";
}

function CallResultSummaryBody({
  sessionId,
  task,
  desktop,
  onDetailLoaded,
}: {
  sessionId: string;
  task: TaskSummary;
  desktop: boolean;
  onDetailLoaded?: (detail: ManagedCallDetail | null) => void;
}): React.JSX.Element {
  const call = task.call!;
  const { detail, error } = useManagedCall(sessionId, call.call_id, call.updated_at_ms);
  useEffect(() => {
    onDetailLoaded?.(detail);
  }, [detail, onDetailLoaded]);
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
    <Box data-task-reader-header sx={{ px: desktop ? 1.5 : 1.75, py: desktop ? 0.85 : 1, borderBottom: 1, borderColor: "divider", flexShrink: 0, display: "flex", flexDirection: "column", gap: 0.5 }}>
      <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
        <Typography variant="subtitle2" noWrap sx={{ flex: 1, minWidth: 0, fontWeight: 700, fontSize: desktop ? "0.875rem" : "0.9375rem" }} title={task.title}>
          {task.title}
        </Typography>
        {task.health && <Typography variant="caption" color="warning.main" sx={{ flexShrink: 0 }}>{task.health}</Typography>}
        <TaskChip task={task} />
        {desktop && (
          <Tooltip title={activeMode === "info" ? "Back to reader (I)" : "Execution details & diagnostics (I)"}>
            <IconButton size="small" color={activeMode === "info" ? "primary" : "default"} data-task-info-toggle aria-label="Task info and diagnostics" aria-pressed={activeMode === "info"} onClick={onToggleInfo}>
              <InfoOutlined fontSize="small" />
            </IconButton>
          </Tooltip>
        )}
      </Stack>
      <Stack direction="row" spacing={0.75} alignItems="center" flexWrap="wrap" useFlexGap>
        {(verdict || modelMeta || elapsed) && (
          <Typography variant="caption" noWrap sx={{ color: "text.secondary", minWidth: 0, flex: "1 1 auto" }}>
            {[verdict, modelMeta, elapsed].filter(Boolean).join(" · ")}
          </Typography>
        )}
        <Stack direction="row" spacing={0.5} alignItems="center" flexWrap="wrap" useFlexGap sx={{ ml: "auto" }}>
          {hasConversation && desktop && (
            <Button size="small" variant={activeMode === "transcript" ? "contained" : "outlined"} disableElevation data-task-mode-transcript startIcon={activeMode === "transcript" ? <SubjectOutlined fontSize="inherit" /> : <TerminalOutlined fontSize="inherit" />} onClick={onToggleTranscript} sx={{ textTransform: "none", minHeight: 26, py: 0, px: 1, fontSize: "0.75rem" }}>
              {activeMode === "transcript" ? "Result summary" : "Full transcript"}
            </Button>
          )}
          {desktop && (
            <Button size="small" variant="outlined" data-task-quote-prompt startIcon={<FormatQuoteOutlined fontSize="inherit" />} onClick={onQuote} sx={{ textTransform: "none", minHeight: 26, py: 0, px: 1, fontSize: "0.75rem" }}>
              Quote to Prompt
            </Button>
          )}
          {task.source === "call" && task.call && desktop && (
            <Tooltip title="Open child call as its own session (O)">
              <IconButton size="small" data-task-open-session aria-label="Open call as session" onClick={onOpenChild}>
                <OpenInNew fontSize="small" />
              </IconButton>
            </Tooltip>
          )}
          {desktop && onLocate && task.anchorSeq !== null && (
            <Tooltip title="Locate in Conversation (Shift+L)">
              <IconButton size="small" data-task-locate aria-label="Locate task in transcript" onClick={onLocate}>
                <MyLocation fontSize="small" />
              </IconButton>
            </Tooltip>
          )}
          {task.canStop && (
            <Button size="small" color="error" variant="outlined" data-task-detail-stop disabled={stopStopping} startIcon={<StopCircleOutlined fontSize="inherit" />} onClick={onStopClick} sx={{ textTransform: "none", minHeight: desktop ? 26 : 32, py: 0, px: 1, fontSize: "0.75rem" }}>
              {stopStopping ? "Stopping…" : "Stop"}
            </Button>
          )}
          {task.attention === "failed_unseen" && (
            <Button size="small" variant="outlined" data-task-mark-seen onClick={(): void => acknowledgeTask(sessionId, task)} sx={{ textTransform: "none", minHeight: desktop ? 26 : 32, py: 0, px: 1, fontSize: "0.75rem" }}>
              Mark as seen
            </Button>
          )}
        </Stack>
      </Stack>
      {stopMessage && <Typography variant="caption" color="warning.main" role="status">{stopMessage}</Typography>}
    </Box>
  );
}

function TaskReaderSummaryBody({
  sessionId,
  task,
  desktop,
  hasConversation,
  originKey,
  onDetailLoaded,
  onAnswerApproval,
  onOpenTranscript,
  onQuote,
  onOpenOriginTool,
}: {
  sessionId: string;
  task: TaskSummary;
  desktop: boolean;
  hasConversation: boolean;
  originKey: string | null;
  onDetailLoaded: (detail: ManagedCallDetail | null) => void;
  onAnswerApproval: () => void;
  onOpenTranscript: () => void;
  onQuote: () => void;
  onOpenOriginTool?: ((toolKey: string) => void) | undefined;
}): React.JSX.Element {
  return (
    <Box data-task-detail={task.key} sx={{ minWidth: 0 }}>
      {task.source === "approval"
        ? (
          <Stack spacing={1.25}>
            <Typography variant="body2">{task.subtitle || "A managed call is waiting for your admission decision."}</Typography>
            <Box><Button size="small" variant="contained" color="warning" data-task-answer-approval onClick={onAnswerApproval} sx={{ textTransform: "none" }}>Review request</Button></Box>
          </Stack>
        )
        : task.source === "call" && task.call
        ? <CallResultSummaryBody sessionId={sessionId} task={task} desktop={desktop} onDetailLoaded={onDetailLoaded} />
        : task.source === "subagent" && task.subagent
        ? <SubagentDetail subagent={task.subagent} resumed={task.subagent.revision > 1 && task.subagent.terminalSeq !== null} />
        : null}
      <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap sx={{ mt: 2 }}>
        {hasConversation && (
          <Button size="small" variant="outlined" data-task-open-transcript startIcon={<TerminalOutlined fontSize="small" />} onClick={onOpenTranscript} sx={{ textTransform: "none" }}>
            {isTaskEnded(task) ? "Full transcript" : "Live transcript"}
          </Button>
        )}
        {!desktop && (
          <Button size="small" variant="outlined" data-task-quote-prompt startIcon={<FormatQuoteOutlined fontSize="small" />} onClick={onQuote} sx={{ textTransform: "none" }}>
            Quote to Prompt
          </Button>
        )}
        {originKey && onOpenOriginTool && (
          <Button size="small" variant="text" data-task-open-origin onClick={(): void => onOpenOriginTool(originKey)} sx={{ textTransform: "none" }}>
            Open originating tool card
          </Button>
        )}
      </Stack>
    </Box>
  );
}

export function TaskResultReader({
  sessionId,
  task,
  background,
  subagentTranscript,
  desktop,
  modeOverride,
  onModeChange,
  onAnswerApproval,
  onLocate,
  onOpenOriginTool,
  onQuoted,
  onOpenChildSession,
  extraInfoFooter,
  bottomInset,
}: {
  sessionId: string;
  task: TaskSummary;
  background: BackgroundTasks;
  subagentTranscript: SubagentTranscriptState;
  desktop: boolean;
  modeOverride?: TaskReaderViewMode | undefined;
  onModeChange?: ((mode: TaskReaderViewMode) => void) | undefined;
  onAnswerApproval: () => void;
  onLocate?: (() => void) | undefined;
  onOpenOriginTool?: ((toolKey: string) => void) | undefined;
  onQuoted?: (() => void) | undefined;
  onOpenChildSession?: ((childSessionId: string) => void) | undefined;
  extraInfoFooter?: React.ReactNode;
  bottomInset?: string | undefined;
}): React.JSX.Element {
  const hasConversation = task.source === "call" || (task.source === "subagent" && subagentTranscript.observed);
  const [internalMode, setInternalMode] = useState<{ key: string; mode: TaskReaderViewMode }>(() => ({ key: task.key, mode: defaultReaderMode(task, subagentTranscript.observed) }));
  const activeMode: TaskReaderViewMode = modeOverride ?? (internalMode.key === task.key ? internalMode.mode : defaultReaderMode(task, subagentTranscript.observed));
  const setMode = (next: TaskReaderViewMode): void => {
    setInternalMode({ key: task.key, mode: next });
    onModeChange?.(next);
  };
  const [loadedCallDetail, setLoadedCallDetail] = useState<ManagedCallDetail | null>(null);
  useEffect(() => {
    setLoadedCallDetail(null);
  }, [task.key]);
  const stops = useStopStates();
  const stop = stops.get(task.key);
  const [confirmingStop, setConfirmingStop] = useState(false);
  const stopMessage = stop && stop.outcome !== "stopping" ? (stop.message ? `${STOP_MESSAGE[stop.outcome]}: ${stop.message}` : STOP_MESSAGE[stop.outcome]) : null;
  const scrollRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (node) node.scrollTop = savedTaskReaderScroll(sessionId, task.key);
  }, [sessionId, task.key, activeMode]);

  const originKey = task.call ? originToolKey(background.items, task.call.request_id) : null;
  const quoteToPrompt = (): void => {
    const excerpt = loadedCallDetail?.result?.text ?? (loadedCallDetail?.result?.structured ? JSON.stringify(loadedCallDetail.result.structured, null, 2) : null);
    appendDraftText(sessionId, formatTaskQuote(task, excerpt));
    notify("Quoted to Prompt", "info");
    onQuoted?.();
  };
  const openChild = (): void => {
    if (!task.call) return;
    const childId = callConversationId(task.call);
    if (onOpenChildSession) onOpenChildSession(childId);
    else setActiveSessionId(childId);
  };

  return (
    <Box data-task-reader={task.key} data-task-reader-mode={activeMode} sx={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
      <TaskReaderHeader
        sessionId={sessionId}
        task={task}
        desktop={desktop}
        activeMode={activeMode}
        hasConversation={hasConversation}
        stopStopping={stop?.outcome === "stopping"}
        stopMessage={stopMessage}
        onToggleInfo={(): void => setMode(activeMode === "info" ? defaultReaderMode(task, subagentTranscript.observed) : "info")}
        onToggleTranscript={(): void => setMode(activeMode === "transcript" ? "result" : "transcript")}
        onQuote={quoteToPrompt}
        onOpenChild={openChild}
        onLocate={onLocate}
        onStopClick={(): void => setConfirmingStop(true)}
      />
      {activeMode === "transcript" && hasConversation
        ? (
          <Box data-task-transcript-host sx={{ flex: 1, minHeight: 0, minWidth: 0, display: "flex", position: "relative" }}>
            {task.source === "call" && task.call
              ? <CallConversation call={task.call} desktop={desktop} onDetails={(): void => setMode("info")} />
              : task.source === "subagent" && task.subagent && <SubagentConversation sessionId={sessionId} subagent={task.subagent} state={subagentTranscript} desktop={desktop} />}
          </Box>
        )
        : (
          <Box ref={scrollRef} data-task-reader-scroll onScroll={(event): void => rememberTaskReaderScroll(sessionId, task.key, event.currentTarget.scrollTop)} sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 2, py: 1.5, pb: bottomInset ?? (desktop ? 2 : "var(--cowboy-sheet-dock-clearance, 88px)"), ...desktopScrollbarSx }}>
            {activeMode === "info"
              ? (
                <Box data-task-detail={task.key} data-task-info-panel sx={{ minWidth: 0 }}>
                  {task.source === "call" && task.call
                    ? <CallDetailView parent={sessionId} call={task.call} desktop={desktop} onOpenChild={openChild} />
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
              )
              : (
                <TaskReaderSummaryBody
                  sessionId={sessionId}
                  task={task}
                  desktop={desktop}
                  hasConversation={hasConversation}
                  originKey={originKey}
                  onDetailLoaded={setLoadedCallDetail}
                  onAnswerApproval={onAnswerApproval}
                  onOpenTranscript={(): void => setMode("transcript")}
                  onQuote={quoteToPrompt}
                  onOpenOriginTool={onOpenOriginTool}
                />
              )}
            <ScrollEdges />
          </Box>
        )}
      <SubagentStopConfirm open={confirmingStop && task.canStop} title={task.title} noun={task.source === "call" ? "call" : "subagent"} onClose={(): void => setConfirmingStop(false)} onConfirm={(): void => { setConfirmingStop(false); requestTaskStop(sessionId, task); }} />
    </Box>
  );
}
