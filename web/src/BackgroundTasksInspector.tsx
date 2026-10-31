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
import {
  KeyboardArrowDown,
  MyLocation,
  OpenInNew,
  StopCircleOutlined,
} from "@mui/icons-material";
import {
  acknowledgeTask,
  groupTasks,
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
import { DesktopShortcutBar } from "./desktop/DesktopShortcutBar";
import { desktopScrollbarSx } from "./desktop/desktopScrollbar";
import { CallConversation, CallDetailView } from "./ManagedCallsDock";
import { callConversationId } from "./managedCalls";
import { Sheet } from "./Sheet";
import { SubagentDetail, SubagentStopConfirm } from "./SubagentCard";
import { TaskChip } from "./TaskRow";
import {
  type BackgroundTasks,
  requestTaskStop,
  STOP_MESSAGE,
  useStopStates,
} from "./useBackgroundTasks";

function typingTarget(event: ReactKeyboardEvent): boolean {
  if (event.nativeEvent.isComposing || event.keyCode === 229) return true;
  const target = event.target as HTMLElement | null;
  return !!target?.closest?.("input, textarea, select, [contenteditable='true'], [contenteditable='']");
}

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
  onOpenConversation,
  onAnswerApproval,
}: {
  task: TaskSummary;
  sessionId: string;
  desktop: boolean;
  resumed: boolean;
  onOpenConversation: (task: TaskSummary) => void;
  onAnswerApproval: () => void;
}): React.JSX.Element {
  const stops = useStopStates();
  const stop = stops.get(task.key);
  const [confirming, setConfirming] = useState(false);
  const stopMessage = stop && stop.outcome !== "stopping"
    ? (stop.message ? `${STOP_MESSAGE[stop.outcome]}: ${stop.message}` : STOP_MESSAGE[stop.outcome])
    : null;
  return (
    <Box data-task-detail={task.key} sx={{ color: "text.primary", typography: "body2", minWidth: 0 }}>
      <Stack direction="row" alignItems="center" spacing={1} sx={{ mb: 1, minWidth: 0 }}>
        {/* Mobile names the task in the sheet's title already. */}
        <Typography
          variant="subtitle1"
          sx={{ fontWeight: 750, flex: 1, minWidth: 0, ...(desktop ? {} : { visibility: "hidden" }) }}
          noWrap
          aria-hidden={!desktop}
        >
          {task.title}
        </Typography>
        {task.health && <Typography variant="caption" color="warning.main">{task.health}</Typography>}
        <TaskChip task={task} size="medium" />
      </Stack>
      {task.subtitle && task.source !== "subagent" && (
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>{task.subtitle}</Typography>
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
        <SubagentDetail subagent={task.subagent} resumed={resumed} />
      )}
      {task.source === "call" && task.call && (
        <Stack spacing={1}>
          {!desktop && (
            <Box>
              <Button
                size="small"
                startIcon={<OpenInNew />}
                data-task-open-conversation
                onClick={(): void => onOpenConversation(task)}
                sx={{ textTransform: "none" }}
              >
                Conversation
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
}: {
  sessionId: string;
  desktop: boolean;
  background: BackgroundTasks;
  /** Scroll the transcript to the row anchored at this key. */
  onLocate?: ((key: string) => void) | undefined;
}): React.JSX.Element | null {
  const state = useInspectorState();
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
    const key = taskKeyFromHash(globalThis.location?.hash ?? "");
    if (key && byKey.has(key) && inspectorState().sessionId === null) {
      openBackgroundTask(sessionId, key, "link");
    }
  }, [sessionId, byKey]);

  // Opening details acknowledges an unseen failure on this device.
  useEffect(() => {
    if (open && selected && (desktop || level !== "list")) acknowledgeTask(selected);
  }, [open, selected, desktop, level]);

  // Mobile: list -> detail -> conversation, one history entry per level, so
  // the system Back walks up a level and finally returns to the transcript.
  useLayoutEffect(() => {
    if (!open || desktop) return;
    setLevel(state.key ? "detail" : "list");
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
        setLevel("detail");
      }
    };
    globalThis.addEventListener("popstate", onPop);
    return () => globalThis.removeEventListener("popstate", onPop);
  }, [open, desktop]);

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
      setLevel("detail");
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
    setLevel("conversation");
    globalThis.history?.pushState({ cowboyTasks: 3 }, "", taskHash(task.key));
    depth.current = 3;
  };
  const answerApproval = (): void => {
    close();
    globalThis.requestAnimationFrame?.(() => {
      document.querySelector<HTMLElement>(
        "[data-desktop-approval-dock] button, [data-call-approval] button",
      )?.focus();
    });
  };
  const locate = (key: string): void => {
    close();
    onLocate?.(key);
  };
  const keys = (event: ReactKeyboardEvent<HTMLElement>): void => {
    if (event.metaKey || event.ctrlKey || event.altKey || typingTarget(event)) return;
    // A confirmation (Stop) owns its own keys.
    if ((event.target as HTMLElement).closest?.("[role='dialog'] [role='dialog']")) return;
    switch (event.key) {
      case "j":
      case "]":
        event.preventDefault();
        step(1);
        return;
      case "k":
      case "[":
        event.preventDefault();
        step(-1);
        return;
      case "o":
        if (selected?.source === "call") {
          event.preventDefault();
          openConversation(selected);
        }
        return;
      case "L":
        if (selected && selected.anchorSeq !== null) {
          event.preventDefault();
          locate(selected.key);
        }
        return;
      case "x":
      case "X":
        if (selected?.canStop) {
          event.preventDefault();
          const detail = document.querySelector<HTMLElement>(`[data-task-detail="${CSS.escape(selected.key)}"]`);
          (detail?.querySelector<HTMLButtonElement>("[data-task-detail-stop]") ??
            [...(detail?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
              .find((button) => /^stop call$/i.test(button.textContent?.trim() ?? "")))?.click();
        }
        return;
      case "Enter":
        if (selected?.source === "approval") {
          event.preventDefault();
          answerApproval();
        }
        return;
      case "h":
      case "Escape":
        event.preventDefault();
        close();
        return;
    }
  };
  const detail = selected
    ? (
      <TaskDetail
        task={selected}
        sessionId={sessionId}
        desktop={desktop}
        resumed={resumedChildren.has(selected.key)}
        onOpenConversation={openConversation}
        onAnswerApproval={answerApproval}
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
        onClose={close}
        maxWidth={false}
        fullWidth
        aria-labelledby="background-tasks-title"
        PaperProps={{
          sx: {
            width: "calc(100vw - 32px)",
            maxWidth: 1600,
            height: "calc(100dvh - 32px)",
            maxHeight: 1100,
            m: 2,
            overflow: "hidden",
            borderRadius: 2.5,
            backgroundImage: "none",
          },
        }}
      >
        <Box
          data-background-tasks-inspector={sessionId}
          data-desktop-keys="own"
          onKeyDown={keys}
          sx={{ height: "100%", minHeight: 0, display: "grid", gridTemplateRows: "auto minmax(0, 1fr) auto" }}
        >
          <Box
            sx={{
              px: 2.5,
              py: 1.25,
              display: "grid",
              gridTemplateColumns: "clamp(280px, 24vw, 380px) minmax(0, 1fr)",
              alignItems: "center",
              gap: 2.5,
              borderBottom: 1,
              borderColor: "divider",
            }}
          >
            <Box>
              <Typography id="background-tasks-title" variant="subtitle1" sx={{ fontWeight: 800 }}>
                Background tasks
              </Typography>
              <Typography variant="caption" color="text.secondary">
                Subagents and calls this session started
              </Typography>
            </Box>
            <Stack direction="row" alignItems="center" spacing={1.25} sx={{ minWidth: 0, justifyContent: "flex-end" }}>
              {selected && selected.anchorSeq !== null && (
                <Button
                  onClick={(): void => locate(selected.key)}
                  startIcon={<MyLocation fontSize="small" />}
                  aria-label="Locate task in transcript"
                  sx={{ textTransform: "none" }}
                >
                  Locate
                </Button>
              )}
              <Button
                onClick={close}
                endIcon={<KeyboardArrowDown />}
                sx={{ textTransform: "none", flexShrink: 0 }}
              >
                Close
              </Button>
            </Stack>
          </Box>
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
            </Box>
            <Box
              sx={{
                minHeight: 0,
                display: "flex",
                flexDirection: selected?.source === "call" ? "row" : "column",
                gap: 2,
                overflow: "hidden",
              }}
            >
              {selected?.source === "call" && selected.call && (
                <CallConversation call={selected.call} desktop detailsOpen onDetails={(): void => undefined} />
              )}
              <Box
                sx={{
                  ...(selected?.source === "call"
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
              </Box>
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
  return (
    <Sheet
      open
      onClose={close}
      title={mobileTitle}
      onBack={level === "list"
        ? undefined
        : (): void => globalThis.history?.back()}
      fill={level === "conversation"}
      forceSheet
      cover
      portal
    >
      <Box
        data-background-tasks-inspector={sessionId}
        data-mobile-task-level={level}
        sx={{
          pb: "calc(16px + env(safe-area-inset-bottom, 0px))",
          color: "text.primary",
          typography: "body2",
          ...(level === "conversation" ? { flex: 1, display: "flex", minHeight: 0 } : {}),
        }}
      >
        {level === "list" || !selected
          ? (
            <Stack>
              <TaskList tasks={tasks} selected={null} desktop={false} onSelect={select} />
              {legacySwitch}
            </Stack>
          )
          : level === "conversation" && selected.call
          ? <CallConversation call={selected.call} desktop={false} onDetails={(): void => globalThis.history?.back()} />
          : detail}
      </Box>
    </Sheet>
  );
}
