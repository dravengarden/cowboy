import { ScrollEdges } from "@cowboy/app-shell";
import { READER_TOP_FROST_FADE, readerTopFrost } from "./readerTopEdge";
// The background-tasks inspector (docs/background-tasks.md §6.2): the Tool
// inspector's frame — a Desktop Dialog with a rail beside the details, a
// Mobile cover Sheet — listing every TaskSummary grouped Needs you / Active /
// Ended. Details route by source to the views that already exist: a subagent
// shows its card body, a Call its conversation and details, a pending
// admission jumps to the approval surface. There is no universal detail view.

import {
  type KeyboardEvent as ReactKeyboardEvent,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import {
  Box,
  Button,
  ButtonBase,
  Dialog,
  Stack,
  Typography,
} from "@mui/material";
import { StopCircleOutlined } from "@mui/icons-material";
import {
  acknowledgeTask,
  groupTasks,
  originToolKey,
  setCallsDockLegacy,
  TASK_SECTION_LABEL,
  taskHash,
  taskKeyFromHash,
  type TaskSummary,
} from "./backgroundTasks";
import {
  closeBackgroundTasks,
  inspectorState,
  openBackgroundTask,
  OPEN_BACKGROUND_TASKS_EVENT,
  selectBackgroundTask,
  useInspectorState,
} from "./backgroundTaskInspectorState";
import { setActiveSessionId } from "./controlPlane";
import { desktopDialogSlotProps, InstantModalTransition, isImeBlockedEscape } from "./desktop/DesktopModal";
import { DesktopShortcutBar } from "./desktop/DesktopShortcutBar";
import { DesktopTasksHeader } from "./DesktopTasksHeader";
import { handleBackgroundTasksKeyDown } from "./backgroundTasksKeyboard";
import { desktopScrollbarSx } from "./desktop/desktopScrollbar";
import { CallConversation, CallDetailView } from "./ManagedCallsDock";
import { callConversationId } from "./managedCalls";
import { Sheet } from "./Sheet";
import { SubagentDetail, SubagentStopConfirm } from "./SubagentCard";
import { SubagentConversation, useTaskConversation } from "./SubagentTranscript";
import { TaskChip } from "./TaskRow";
import { taskDockContext } from "./taskDockContext";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import {
  type BackgroundTasks,
  requestTaskStop,
  STOP_MESSAGE,
  useStopStates,
} from "./useBackgroundTasks";

function Diagnostics({ task }: { task: TaskSummary }): React.JSX.Element {
  const rows: [string, string][] = [
    ["source", task.diag.source],
    ["entity", task.diag.entity],
    ["execution", task.diag.execution ?? "none (latest observed state)"],
    ["incarnation", task.diag.incarnation ?? "—"],
    ["source_revision", task.diag.sourceRevision ?? "—"],
    ["observed_seq", task.diag.observedSeq === null ? "—" : String(task.diag.observedSeq)],
    ["anchor_seq", task.diag.anchorSeq === null ? "—" : String(task.diag.anchorSeq)],
    ["reconciled_by", task.diag.reconciledBy ?? "—"],
    [
      "reconciled_at",
      task.diag.reconciledAt === null ? "—" : new Date(task.diag.reconciledAt).toLocaleTimeString(),
    ],
  ];
  return (
    <Box
      component="details"
      data-task-diagnostics
      sx={{ mt: 2, color: "text.secondary", "& summary": { cursor: "pointer", typography: "caption", fontWeight: 650 } }}
    >
      <summary>Diagnostics</summary>
      <Box
        component="dl"
        sx={{
          m: 0,
          mt: 0.75,
          display: "grid",
          gridTemplateColumns: "max-content minmax(0, 1fr)",
          columnGap: 1.5,
          rowGap: 0.25,
          typography: "caption",
          fontFamily: "monospace",
        }}
      >
        {rows.map(([label, value]) => (
          <Box key={label} sx={{ display: "contents" }}>
            <Box component="dt">{label}</Box>
            <Box component="dd" data-diagnostic={label} sx={{ m: 0, overflowWrap: "anywhere" }}>{value}</Box>
          </Box>
        ))}
      </Box>
    </Box>
  );
}

function TaskList({
  tasks,
  selected,
  desktop,
  onSelect,
}: {
  tasks: readonly TaskSummary[];
  selected: string | null;
  desktop: boolean;
  onSelect: (key: string) => void;
}): React.JSX.Element {
  const sections = groupTasks(tasks);
  if (sections.length === 0) {
    return (
      <Typography variant="body2" color="text.secondary" sx={{ p: 2 }}>
        No background tasks in this session.
      </Typography>
    );
  }
  return (
    <Stack component="nav" aria-label="Background tasks" spacing={1.5} sx={{ p: desktop ? 1.5 : 0 }}>
      {sections.map(({ section, tasks: sectionTasks }) => (
        <Box key={section} data-task-section={section}>
          <Typography
            variant="caption"
            sx={{ display: "block", px: 1, pb: 0.5, color: "text.secondary", fontWeight: 700 }}
          >
            {TASK_SECTION_LABEL[section]}
          </Typography>
          <Stack spacing={0.25} role="listbox" aria-label={TASK_SECTION_LABEL[section]}>
            {sectionTasks.map((task) => (
              <ButtonBase
                key={task.key}
                role="option"
                aria-selected={task.key === selected}
                data-task-option={task.key}
                onClick={(): void => onSelect(task.key)}
                sx={{
                  display: "flex",
                  alignItems: "center",
                  gap: 1,
                  px: 1,
                  py: 0.75,
                  minHeight: desktop ? 40 : 48,
                  borderRadius: 1,
                  textAlign: "left",
                  justifyContent: "flex-start",
                  bgcolor: task.key === selected ? "action.selected" : "transparent",
                  "&:hover, &:focus-visible": { bgcolor: "action.hover" },
                }}
              >
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <Typography variant="body2" noWrap sx={{ fontWeight: 650 }}>
                    {task.parentKey ? "↳ " : ""}{task.title}
                    {task.childCount > 0 ? ` (+${String(task.childCount)})` : ""}
                  </Typography>
                  {task.subtitle && (
                    <Typography variant="caption" noWrap sx={{ display: "block", color: "text.secondary" }}>
                      {task.subtitle}
                    </Typography>
                  )}
                </Box>
                {task.health && (
                  <Typography variant="caption" color="warning.main" sx={{ flexShrink: 0 }}>
                    {task.health}
                  </Typography>
                )}
                <TaskChip task={task} />
              </ButtonBase>
            ))}
          </Stack>
        </Box>
      ))}
    </Stack>
  );
}

function TaskDetail({
  task,
  sessionId,
  desktop,
  resumed,
  observed = false,
  onAnswerApproval,
  onOpenOrigin,
}: {
  task: TaskSummary;
  sessionId: string;
  desktop: boolean;
  resumed: boolean;
  /** Its full transcript is shown beside or before these details. */
  observed?: boolean;
  onAnswerApproval: () => void;
  /** The MCP tool card that started this Call, when one is loaded. */
  onOpenOrigin?: (() => void) | undefined;
}): React.JSX.Element {
  const stops = useStopStates();
  const stop = stops.get(task.key);
  const [confirming, setConfirming] = useState(false);
  const stopMessage = stop && stop.outcome !== "stopping"
    ? (stop.message ? `${STOP_MESSAGE[stop.outcome]}: ${stop.message}` : STOP_MESSAGE[stop.outcome])
    : null;
  return (
    <Box data-task-detail={task.key} sx={{ color: "text.primary", typography: "body2", minWidth: 0 }}>
      <Stack
        direction="row"
        alignItems="center"
        spacing={1}
        // The pill floors at 24px while its slot follows rem, so the row
        // reserves the pill's true height: at a reduced app font scale it
        // would otherwise overflow the row and be clipped by the scroller.
        sx={{ mb: 1, minWidth: 0, minHeight: `max(24px, ${desktop ? "2rem" : "1.5rem"})` }}
      >
        {/* Mobile names the task in the sheet's title already. */}
        <Typography
          variant="subtitle1"
          sx={{ fontWeight: 750, flex: 1, minWidth: 0, overflowWrap: "anywhere" }}
        >
          {task.title}
        </Typography>
        {task.health && <Typography variant="caption" color="warning.main">{task.health}</Typography>}
        {/* Mobile stacks this beside a Call's own state pill: one pill size. */}
        <TaskChip task={task} size={desktop ? "medium" : "small"} />
      </Stack>
      {task.subtitle && task.source !== "subagent" && (
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>{task.subtitle}</Typography>
      )}
      {task.attention === "failed_unseen" && (
        <Button size="small" data-task-mark-seen onClick={() => acknowledgeTask(sessionId, task)}>
          Mark as seen
        </Button>
      )}
      {/* A Call's details carry their own Stop and Open as session. */}
      {((task.canStop && task.source !== "call") || stopMessage) && (
        <Stack direction="row" alignItems="center" spacing={1} sx={{ mb: 1 }}>
          {task.canStop && task.source !== "call" && (
            <Button
              size="small"
              color="error"
              variant="outlined"
              data-task-detail-stop
              disabled={stop?.outcome === "stopping"}
              startIcon={<StopCircleOutlined />}
              onClick={(): void => setConfirming(true)}
              sx={{ textTransform: "none" }}
            >
              {stop?.outcome === "stopping" ? "Stopping…" : "Stop"}
            </Button>
          )}
          {stopMessage && (
            <Typography variant="caption" color="warning.main" data-task-stop-outcome={stop?.outcome}>
              {stopMessage}
            </Typography>
          )}
        </Stack>
      )}
      {task.source === "subagent" && task.subagent && (
        <SubagentDetail subagent={task.subagent} resumed={resumed && !observed} summaryOnly={observed} />
      )}
      {task.source === "call" && task.call && (
        <Stack spacing={1}>
          {onOpenOrigin && (
            <Box>
              <Button
                size="small"
                data-task-open-origin
                onClick={onOpenOrigin}
                sx={{ textTransform: "none" }}
              >
                Show the tool call that started it
              </Button>
            </Box>
          )}

          <CallDetailView
            parent={sessionId}
            call={task.call}
            desktop={desktop}
            onOpenChild={(child): void => {
              closeBackgroundTasks();
              setActiveSessionId(child);
            }}
          />
        </Stack>
      )}
      {task.source === "approval" && task.approval && (
        <Stack spacing={1}>
          <Typography variant="body2">
            {task.approval.requests === 1
              ? "One call waits for your approval."
              : `${String(task.approval.requests)} calls wait for your approval.`}
          </Typography>
          {task.approval.items.map((item, index) => (
            <Typography key={item.approval_request_id ?? String(index)} variant="caption" color="text.secondary">
              {item.summary}
            </Typography>
          ))}
          <Box>
            <Button variant="contained" size="small" data-task-answer-approval onClick={onAnswerApproval} sx={{ textTransform: "none" }}>
              Answer
            </Button>
          </Box>
        </Stack>
      )}
      <Diagnostics task={task} />
      <SubagentStopConfirm
        open={confirming && task.canStop}
        title={task.title}
        noun={task.source === "call" ? "call" : "subagent"}
        onClose={(): void => setConfirming(false)}
        onConfirm={(): void => {
          setConfirming(false);
          requestTaskStop(sessionId, task);
        }}
      />
    </Box>
  );
}

/** Children that went idle and then ran again: their snapshot shows the
 *  latest run only (§3.1). Device-local, for this page lifetime. */
const resumedChildren = new Set<string>();
const terminalChildren = new Set<string>();

function noteRuns(tasks: readonly TaskSummary[]): void {
  for (const task of tasks) {
    if (task.source !== "subagent" || !task.subagent) continue;
    if (task.subagent.state === "idle") terminalChildren.add(task.key);
    else if (terminalChildren.has(task.key)) resumedChildren.add(task.key);
  }
}

function focusAfterClose(target: HTMLElement | null, fromBar: boolean): void {
  globalThis.requestAnimationFrame?.(() => {
    if (fromBar || !target || !target.isConnected) {
      const editor = document.querySelector<HTMLElement>(
        "[data-desktop-region='prompt.composer'] .cm-content, [data-composer-editor] .cm-content",
      );
      if (editor) {
        editor.focus({ preventScroll: true });
        return;
      }
    }
    target?.focus({ preventScroll: true });
  });
}

type MobileLevel = "list" | "detail" | "conversation";

export function BackgroundTasksInspector({
  sessionId,
  desktop,
  background,
  onLocate,
  onOpenTool,
}: {
  sessionId: string;
  desktop: boolean;
  background: BackgroundTasks;
  /** Scroll the transcript to the row anchored at this key. */
  onLocate?: ((key: string) => void) | undefined;
  /** Open a transcript tool card in the Tool inspector (a Call's origin). */
  onOpenTool?: ((toolKey: string) => void) | undefined;
}): React.JSX.Element | null {
  const state = useInspectorState();
  const { data: accounts } = useSubscriptionAccounts();
  const open = state.sessionId === sessionId;
  const { tasks, byKey } = background;
  noteRuns(tasks);
  const ordered = useMemo(() => groupTasks(tasks).flatMap((section) => section.tasks), [tasks]);
  // Selection is stable by key while groups reorder; a task that leaves the
  // list keeps its last summary visible until the person moves on.
  const lastSeen = useRef(new Map<string, TaskSummary>());
  for (const task of tasks) lastSeen.current.set(task.key, task);
  const selectedKey = open ? state.key ?? (desktop ? ordered[0]?.key ?? null : null) : null;
  const selected = selectedKey ? byKey.get(selectedKey) ?? lastSeen.current.get(selectedKey) ?? null : null;
  const [level, setLevel] = useState<MobileLevel>("list");
  const depth = useRef(0);
  const { child, conversation } = useTaskConversation(sessionId, open ? selected : null, desktop, level, setLevel, depth);
  const consumedDeepLink = useRef<string | null>(null);
  const inspectorRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open || !desktop) return undefined;
    const frame = requestAnimationFrame(() =>
      inspectorRef.current?.focus({ preventScroll: true })
    );
    return () => cancelAnimationFrame(frame);
  }, [open, desktop]);

  // `␣G` and deep links open this session's inspector.
  useEffect(() => {
    const onOpen = (event: Event): void => {
      const detail = (event as CustomEvent<{ sessionId?: string; key?: string | null }>).detail;
      if (detail?.sessionId !== undefined && detail.sessionId !== sessionId) return;
      openBackgroundTask(sessionId, detail?.key ?? null, "bar");
    };
    globalThis.addEventListener(OPEN_BACKGROUND_TASKS_EVENT, onOpen);
    return () => globalThis.removeEventListener(OPEN_BACKGROUND_TASKS_EVENT, onOpen);
  }, [sessionId]);
  useEffect(() => {
    // A task hash is an initial navigation intent, not an instruction to
    // reopen on every task refresh. Close renders before history.go finishes.
    if (consumedDeepLink.current === sessionId) return;
    if (open) {
      consumedDeepLink.current = sessionId;
      return;
    }
    const key = taskKeyFromHash(globalThis.location?.hash ?? "");
    if (key && byKey.has(key) && inspectorState().sessionId === null) {
      consumedDeepLink.current = sessionId;
      openBackgroundTask(sessionId, key, "link");
    }
  }, [sessionId, byKey, open]);

  // Opening details acknowledges an unseen failure. A notification's link only
  // opens them: reaching a row is not seeing it, so it asks for an explicit
  // Mark as seen (docs/task-notifications.md#acknowledgement).
  useEffect(() => {
    if (open && selected && state.origin !== "link" && (desktop || level !== "list")) {
      acknowledgeTask(sessionId, selected);
    }
  }, [open, selected, desktop, level, sessionId, state.origin]);

  // Mobile calls open their transcript first, with details one step deeper.
  // Other sources retain their own detail view. One entry per level, so
  // the system Back walks up a level and finally returns to the transcript.
  useLayoutEffect(() => {
    if (!open || desktop) return;
    setLevel(state.key ? (selected?.source === "call" ? "conversation" : "detail") : "list");
    const entries = state.key ? 2 : 1;
    if (globalThis.history) {
      globalThis.history.pushState({ cowboyTasks: 1 }, "");
      if (state.key) globalThis.history.pushState({ cowboyTasks: 2 }, "", taskHash(state.key));
    }
    depth.current = entries;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, desktop, state.revision]);
  useEffect(() => {
    if (!open || desktop) return undefined;
    const onPop = (event: PopStateEvent): void => {
      const target = (event.state as { cowboyTasks?: number } | null)?.cowboyTasks ?? 0;
      depth.current = target;
      if (target <= 0) {
        focusAfterClose(closeBackgroundTasks(), false);
        return;
      }
      if (target === 1) {
        setLevel("list");
        selectBackgroundTask(null);
      } else if (target === 2) {
        setLevel(conversation ? "conversation" : "detail");
      } else if (target === 3) {
        setLevel("detail");
      }
    };
    globalThis.addEventListener("popstate", onPop);
    return () => globalThis.removeEventListener("popstate", onPop);
  }, [open, desktop, conversation]);

  if (!open) return null;

  const close = (): void => {
    if (!desktop && depth.current > 0 && globalThis.history) {
      const back = depth.current;
      depth.current = 0;
      globalThis.history.go(-back);
    }
    const origin = state.origin;
    focusAfterClose(closeBackgroundTasks(), origin === "bar");
  };
  const select = (key: string): void => {
    selectBackgroundTask(key);
    if (!desktop) {
      setLevel(byKey.get(key)?.source === "call" ? "conversation" : "detail");
      globalThis.history?.pushState({ cowboyTasks: 2 }, "", taskHash(key));
      depth.current = 2;
    }
  };
  const step = (delta: number): void => {
    const index = selectedKey ? ordered.findIndex((task) => task.key === selectedKey) : -1;
    const next = ordered[Math.max(0, Math.min(ordered.length - 1, index + delta))];
    if (next) selectBackgroundTask(next.key);
  };
  const openConversation = (task: TaskSummary): void => {
    if (task.source !== "call" || !task.call) return;
    if (desktop) {
      closeBackgroundTasks();
      setActiveSessionId(callConversationId(task.call));
      return;
    }
    globalThis.history?.back();
  };
  const answerApproval = (): void => {
    close();
    globalThis.requestAnimationFrame?.(() => {
      document.querySelector<HTMLElement>(
        "[data-desktop-approval-dock] button, [data-call-approval-overlay] button",
      )?.focus();
    });
  };
  const locate = (key: string): void => {
    close();
    onLocate?.(key);
  };
  const keys = (event: ReactKeyboardEvent<HTMLElement>): void =>
    handleBackgroundTasksKeyDown(event, {
      ownNode: inspectorRef.current,
      selected,
      step,
      openConversation: () => { if (selected) openConversation(selected); },
      locate,
      answerApproval,
      close,
    });
  // Only a structured MCP identity links a Call to its tool card (I8).
  const originTool = selected?.source === "call" && selected.call
    ? originToolKey(background.items, selected.call.request_id)
    : null;
  const detail = selected
    ? (
      <TaskDetail
        task={selected}
        sessionId={sessionId}
        desktop={desktop}
        resumed={resumedChildren.has(selected.key)}
        observed={selected.source === "subagent" && child.observed}
        onAnswerApproval={answerApproval}
        onOpenOrigin={originTool && onOpenTool
          ? (): void => {
            close();
            onOpenTool(originTool);
          }
          : undefined}
      />
    )
    : (
      <Typography variant="body2" color="text.secondary">
        Select a task to see its details.
      </Typography>
    );
  const legacySwitch = (
    <Button
      size="small"
      color="inherit"
      data-calls-dock-legacy-switch
      onClick={(): void => setCallsDockLegacy(true)}
      sx={{ textTransform: "none", color: "text.secondary", alignSelf: "flex-start", m: 1 }}
    >
      Use the previous Calls panel
    </Button>
  );

  if (desktop) {
    return createPortal(
      <Dialog
        open
        onClose={(e, r) => { if (!isImeBlockedEscape(e, r)) close(); }}
        maxWidth={false}
        fullWidth
        disableScrollLock
        aria-labelledby="background-tasks-title"
        slots={{ transition: InstantModalTransition }}
        slotProps={desktopDialogSlotProps({
          root: { onKeyDown: keys },
          onClose: close,
          paperSx: {
            width: "calc(100vw - 32px)",
            maxWidth: 1600,
            height: "calc(100dvh - 32px)",
            maxHeight: 1100,
            m: 2,
            overflow: "hidden",
          },
        })}
      >
        <Box
          ref={inspectorRef}
          tabIndex={-1}
          data-background-tasks-inspector={sessionId}
          data-desktop-keys="own"
          data-desktop-modal-keys="own"
          sx={{ height: "100%", minHeight: 0, display: "grid", gridTemplateRows: "auto minmax(0, 1fr) auto", outline: "none" }}
        >
          <DesktopTasksHeader
            onLocate={selected && selected.anchorSeq !== null ? (): void => locate(selected.key) : undefined}
            onClose={close}
          />
          <Box sx={{ minHeight: 0, display: "grid", gridTemplateColumns: "clamp(280px, 24vw, 380px) minmax(0, 1fr)" }}>
            <Box
              sx={{
                minHeight: 0,
                overflowY: "auto",
                borderRight: 1,
                borderColor: "divider",
                display: "flex",
                flexDirection: "column",
                ...desktopScrollbarSx,
              }}
            >
              <TaskList tasks={tasks} selected={selectedKey} desktop onSelect={(key): void => selectBackgroundTask(key)} />
              <Box sx={{ flex: 1 }} />
              {legacySwitch}
            <ScrollEdges /></Box>
            <Box
              sx={{
                minHeight: 0,
                display: "flex",
                flexDirection: conversation ? "row" : "column",
                overflow: "hidden",
              }}
            >
              {selected?.source === "call" && selected.call && (
                <CallConversation call={selected.call} desktop />
              )}
              {conversation && selected?.subagent && (
                <SubagentConversation sessionId={sessionId} subagent={selected.subagent} state={child} desktop />
              )}
              <Box
                sx={{
                  ...(conversation
                    ? { width: "clamp(320px, 26vw, 460px)", flexShrink: 0, borderLeft: 1, borderColor: "divider" }
                    : { flex: 1 }),
                  minHeight: 0,
                  overflowY: "auto",
                  px: { md: 3, xl: 4 },
                  py: 2.5,
                  ...desktopScrollbarSx,
                }}
              >
                {detail}
              <ScrollEdges /></Box>
            </Box>
          </Box>
          <DesktopShortcutBar
            groups={[
              {
                label: "Tasks",
                slots: [
                  { shortcut: "J/K", label: "Task", availability: ordered.length > 1 ? "available" : "inactive" },
                  { shortcut: "[/]", label: "Previous / next", availability: ordered.length > 1 ? "available" : "inactive" },
                ],
              },
              {
                slots: [
                  { shortcut: "O", label: "Open as session", availability: selected?.source === "call" ? "available" : "inactive" },
                  { shortcut: "Shift+L", label: "Locate", availability: selected?.anchorSeq !== null && selected ? "available" : "inactive" },
                  { shortcut: "X", label: "Stop", availability: selected?.canStop ? "available" : "inactive" },
                  { shortcut: "Esc", label: "Close" },
                ],
              },
            ]}
          />
        </Box>
      </Dialog>,
      document.body,
    );
  }

  const mobileTitle = level === "list" || !selected ? "Background tasks" : selected.title;
  const showMobileDetails = (): void => {
    if (!selected) return;
    if (level === "detail") { globalThis.history?.back(); return; }
    setLevel("detail");
    globalThis.history?.pushState({ cowboyTasks: 3 }, "", taskHash(selected.key));
    depth.current = 3;
  };
  return (
    <Sheet
      open
      onClose={close}
      title={mobileTitle}
      islandContext={{ ...taskDockContext(level === "list" ? null : selected, Date.now(), accounts), ...(level === "list" || !selected ? { title: "" } : {}), action: level !== "list" && conversation ? { label: level === "detail" ? "Back to transcript" : "Details", onActivate: showMobileDetails } : undefined }}
      onBack={level === "list"
        ? undefined
        : (): void => globalThis.history?.back()}
      fill={level !== "list" && conversation}
      readerTopBleed={level !== "list" && conversation}
      forceSheet
      cover
      portal
    >
      <Box
        data-background-tasks-inspector={sessionId}
        data-mobile-task-level={level}
        sx={{
          // Transcript owns the floating footer clearance; avoid doubling it here.
          pb: conversation && level !== "list" ? 0 : "calc(16px + env(safe-area-inset-bottom, 0px))",
          color: "text.primary",
          typography: "body2",
          ...(conversation && level !== "list" ? { flex: 1, display: "flex", minHeight: 0, minWidth: 0 } : {}),
        }}
      >
        {level === "list" || !selected
          ? (
            <Stack>
              <TaskList tasks={tasks} selected={null} desktop={false} onSelect={select} />
              {legacySwitch}
            </Stack>
          )
          : conversation
          ? (
            <Box
              data-task-reader
              sx={(theme) => ({
                flex: 1,
                minWidth: 0,
                minHeight: 0,
                display: "flex",
                position: "relative",
                // Details scroll behind the status area and blur there; the
                // Transcript keeps its own paint-only fade (readerTopEdge).
                ...(level === "detail" ? readerTopFrost(theme) : {}),
              })}
            >
              <Box inert={level !== "conversation"} sx={{ flex: 1, minWidth: 0, display: "flex", visibility: level === "conversation" ? "visible" : "hidden" }}>
                {selected.call
                  ? <CallConversation call={selected.call} desktop={false} onDetails={showMobileDetails} />
                  : selected.subagent && <SubagentConversation sessionId={sessionId} subagent={selected.subagent} state={child} desktop={false} />}
              </Box>
              {level === "detail" && (
                <Box
                  data-task-reader-scroll
                  sx={{
                    position: "absolute",
                    inset: 0,
                    overflowY: "auto",
                    px: 2,
                    // At rest the first row sits below the frost; only content
                    // that scrolled under the status area is blurred.
                    pt: `calc(var(--cowboy-reader-top-inset, 0px) + ${READER_TOP_FROST_FADE})`,
                    pb: "var(--cowboy-sheet-dock-clearance, 76px)",
                  }}
                >
                  {detail}
                  <ScrollEdges top={false} />
                </Box>
              )}
            </Box>
          )
          : detail}
      </Box>
    </Sheet>
  );
}
