// Background tasks host mounted beside Transcript.

import { ScrollEdges } from "@cowboy/app-shell";
import { type RefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Box, Dialog } from "@mui/material";
import { acknowledgeTask, taskKeyFromHash, type TaskSummary } from "./backgroundTasks";
import { closeBackgroundTasks, inspectorState, openBackgroundTask, OPEN_BACKGROUND_TASKS_EVENT, selectBackgroundTask, useInspectorState } from "./backgroundTaskInspectorState";
import { type BackgroundTasksKeyContext, useBackgroundTasksKeyboard } from "./backgroundTasksKeyboard";
import { setActiveSessionId } from "./controlPlane";
import { desktopDialogSlotProps, InstantModalTransition, isImeBlockedEscape } from "./desktop/DesktopModal";
import { DesktopShortcutBar } from "./desktop/DesktopShortcutBar";
import { desktopScrollbarSx } from "./desktop/desktopScrollbar";
import { DesktopTasksHeader } from "./DesktopTasksHeader";
import { callConversationId } from "./managedCalls";
import { useSubagentTranscript } from "./SubagentTranscript";
import type { BackgroundTasks } from "./useBackgroundTasks";
import { MobileTasksSheet } from "./tasks/MobileTasksSheet";
import { LegacyCallsSwitch } from "./tasks/TaskDiagnostics";
import { focusTaskApprovalSurface, LOCATE_TASK_EVENT, OPEN_TASK_ORIGIN_TOOL_EVENT, useDesktopTasksHostActive } from "./tasks/taskEvents";
import { TaskResultReader } from "./tasks/TaskResultReader";
import { TaskRoster } from "./tasks/TaskRoster";
import { buildTaskForest, flattenTaskSections } from "./tasks/taskTree";

function StandaloneDesktopTasksContent({
  sessionId,
  background,
  ordered,
  selected,
  selectedKey,
  hideEnded,
  collapsedKeys,
  inspectorRef,
  onToggleHideEnded,
  onToggleFold,
  onSelect,
  onLocate,
  onAnswerApproval,
  onOpenTool,
  onClose,
}: {
  sessionId: string;
  background: BackgroundTasks;
  ordered: readonly TaskSummary[];
  selected: TaskSummary | null;
  selectedKey: string | null;
  hideEnded: boolean;
  collapsedKeys: ReadonlySet<string>;
  inspectorRef: RefObject<HTMLDivElement | null>;
  onToggleHideEnded: () => void;
  onToggleFold: (key: string) => void;
  onSelect: (key: string) => void;
  onLocate: (key: string) => void;
  onAnswerApproval: () => void;
  onOpenTool?: ((toolKey: string) => void) | undefined;
  onClose: () => void;
}): React.JSX.Element {
  const subagentTranscript = useSubagentTranscript(sessionId, selected?.source === "subagent" ? (selected.subagent ?? null) : null);
  return (
    <Box ref={inspectorRef} tabIndex={-1} data-background-tasks-inspector={sessionId} data-desktop-keys="own" data-desktop-modal-keys="own" sx={{ height: "100%", minHeight: 0, display: "grid", gridTemplateRows: "auto minmax(0, 1fr) auto", outline: "none" }}>
      <DesktopTasksHeader onLocate={selected && selected.anchorSeq !== null ? (): void => onLocate(selected.key) : undefined} onClose={onClose} />
      <Box sx={{ minHeight: 0, display: "grid", gridTemplateColumns: "clamp(240px, 18vw, 300px) minmax(0, 1fr)" }}>
        <Box sx={{ minHeight: 0, overflowY: "auto", borderRight: 1, borderColor: "divider", display: "flex", flexDirection: "column", ...desktopScrollbarSx }}>
          <TaskRoster sessionId={sessionId} tasks={background.tasks} calls={background.calls} selectedKey={selectedKey} desktop hideEnded={hideEnded} onToggleHideEnded={onToggleHideEnded} collapsedKeys={collapsedKeys} onToggleFold={onToggleFold} onSelect={onSelect} onRefreshCalls={background.refreshCalls} />
          <Box sx={{ flex: 1 }} />
          <LegacyCallsSwitch onSwitch={onClose} />
          <ScrollEdges />
        </Box>
        <Box sx={{ minHeight: 0, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
          {selected && (
            <TaskResultReader sessionId={sessionId} task={selected} background={background} subagentTranscript={subagentTranscript} desktop onAnswerApproval={onAnswerApproval} onLocate={selected.anchorSeq !== null ? (): void => onLocate(selected.key) : undefined} onOpenOriginTool={onOpenTool ? (toolKey): void => { onClose(); onOpenTool(toolKey); } : undefined} onQuoted={onClose} />
          )}
        </Box>
      </Box>
      <DesktopShortcutBar
        groups={[
          { label: "Tasks", slots: [{ shortcut: "J/K", label: "Task", availability: ordered.length > 1 ? "available" : "inactive" }, { shortcut: "[/]", label: "Previous / next", availability: ordered.length > 1 ? "available" : "inactive" }] },
          { slots: [{ shortcut: "O", label: "Open as session", availability: selected?.source === "call" ? "available" : "inactive" }, { shortcut: "Shift+L", label: "Locate", availability: selected?.anchorSeq !== null && selected ? "available" : "inactive" }, { shortcut: "X", label: "Stop", availability: selected?.canStop ? "available" : "inactive" }, { shortcut: "Esc", label: "Close" }] },
        ]}
      />
    </Box>
  );
}

function StandaloneDesktopTasksDialog({ sessionId, background, onLocate, onOpenTool }: { sessionId: string; background: BackgroundTasks; onLocate?: ((key: string) => void) | undefined; onOpenTool?: ((toolKey: string) => void) | undefined }): React.JSX.Element | null {
  const state = useInspectorState();
  const open = state.sessionId === sessionId;
  const [hideEnded, setHideEnded] = useState(false);
  const [collapsedKeys, setCollapsedKeys] = useState<ReadonlySet<string>>(() => new Set());
  const inspectorRef = useRef<HTMLDivElement>(null);
  const forest = useMemo(() => buildTaskForest(background.tasks), [background.tasks]);
  const ordered = useMemo(() => flattenTaskSections(forest, { collapsedKeys, hideEnded }).map((row) => row.task), [forest, collapsedKeys, hideEnded]);
  const selectedKey = state.key && background.byKey.has(state.key) ? state.key : ordered[0]?.key ?? null;
  const selected = selectedKey ? background.byKey.get(selectedKey) ?? null : null;

  useEffect(() => {
    const onOpen = (event: Event): void => {
      const detail = (event as CustomEvent<{ sessionId?: string }>).detail;
      if (detail?.sessionId && detail.sessionId !== sessionId) return;
      openBackgroundTask(sessionId, null, "command");
    };
    globalThis.addEventListener(OPEN_BACKGROUND_TASKS_EVENT, onOpen);
    return () => globalThis.removeEventListener(OPEN_BACKGROUND_TASKS_EVENT, onOpen);
  }, [sessionId]);

  const consumedDeepLinkRef = useRef<string | null>(null);
  useEffect(() => {
    const rawHash = globalThis.location?.hash ?? "";
    const key = taskKeyFromHash(rawHash);
    if (!key) { consumedDeepLinkRef.current = null; return; }
    const intent = `${sessionId}\u0000${rawHash}`;
    if (consumedDeepLinkRef.current === intent || inspectorState().sessionId === sessionId || !background.byKey.has(key)) return;
    consumedDeepLinkRef.current = intent;
    openBackgroundTask(sessionId, key, "link");
  }, [sessionId, background.byKey]);

  const close = useCallback((): void => {
    const returnFocus = closeBackgroundTasks();
    if (returnFocus?.isConnected) requestAnimationFrame(() => returnFocus.focus({ preventScroll: true }));
  }, []);
  const select = useCallback((key: string): void => {
    selectBackgroundTask(key);
    const picked = background.byKey.get(key);
    if (picked?.attention === "failed_unseen") acknowledgeTask(sessionId, picked);
  }, [background.byKey, sessionId]);
  const step = useCallback((delta: number): void => {
    if (ordered.length === 0) return;
    const index = selected ? ordered.findIndex((item) => item.key === selected.key) : -1;
    const next = ordered[(index === -1 ? 0 : index + delta + ordered.length) % ordered.length];
    if (next) select(next.key);
  }, [ordered, select, selected]);
  const locate = useCallback((key: string): void => { close(); requestAnimationFrame(() => onLocate?.(key)); }, [close, onLocate]);
  const answerApproval = useCallback((): void => {
    const target = selected;
    close();
    if (!target) return;
    requestAnimationFrame(() => { focusTaskApprovalSurface(sessionId, target); });
  }, [close, selected, sessionId]);

  const keyContextRef = useRef<BackgroundTasksKeyContext>({ ownNode: null, selected: null, step: () => undefined, openConversation: () => undefined, locate: () => undefined, answerApproval: () => undefined, close: () => undefined });
  keyContextRef.current = { ownNode: inspectorRef.current, selected, step, openConversation: () => { if (selected?.call) setActiveSessionId(callConversationId(selected.call)); }, locate, answerApproval, close };
  useBackgroundTasksKeyboard({ enabled: open, selectedKey, inspectorRef, contextRef: keyContextRef });

  if (!open) return null;
  return createPortal(
    <Dialog open onClose={(event, reason): void => { if (!isImeBlockedEscape(event, reason)) close(); }} maxWidth={false} fullWidth disableScrollLock aria-labelledby="background-tasks-title" slots={{ transition: InstantModalTransition }} slotProps={desktopDialogSlotProps({ paperSx: { width: "calc(100vw - 24px)", maxWidth: "none", height: "calc(100dvh - 24px)", maxHeight: "none", m: 1.5, overflow: "hidden" } })}>
      <StandaloneDesktopTasksContent sessionId={sessionId} background={background} ordered={ordered} selected={selected} selectedKey={selectedKey} hideEnded={hideEnded} collapsedKeys={collapsedKeys} inspectorRef={inspectorRef} onToggleHideEnded={(): void => setHideEnded((value) => !value)} onToggleFold={(key): void => setCollapsedKeys((prev) => { const next = new Set(prev); if (next.has(key)) next.delete(key); else next.add(key); return next; })} onSelect={select} onLocate={locate} onAnswerApproval={answerApproval} onOpenTool={onOpenTool} onClose={close} />
    </Dialog>,
    document.body,
  );
}

export function BackgroundTasksInspector({ sessionId, desktop, background, onLocate, onOpenTool }: { sessionId: string; desktop: boolean; background: BackgroundTasks; onLocate?: ((key: string) => void) | undefined; onOpenTool?: ((toolKey: string) => void) | undefined }): React.JSX.Element | null {
  const desktopHostActive = useDesktopTasksHostActive(sessionId);
  useEffect(() => {
    const handleLocate = (event: Event): void => {
      const detail = (event as CustomEvent<{ sessionId?: string; key?: string }>).detail;
      if (detail?.sessionId === sessionId && detail.key) requestAnimationFrame(() => onLocate?.(detail.key!));
    };
    const handleOpenTool = (event: Event): void => {
      const detail = (event as CustomEvent<{ sessionId?: string; toolKey?: string }>).detail;
      if (detail?.sessionId === sessionId && detail.toolKey) requestAnimationFrame(() => onOpenTool?.(detail.toolKey!));
    };
    globalThis.addEventListener(LOCATE_TASK_EVENT, handleLocate);
    globalThis.addEventListener(OPEN_TASK_ORIGIN_TOOL_EVENT, handleOpenTool);
    return () => {
      globalThis.removeEventListener(LOCATE_TASK_EVENT, handleLocate);
      globalThis.removeEventListener(OPEN_TASK_ORIGIN_TOOL_EVENT, handleOpenTool);
    };
  }, [sessionId, onLocate, onOpenTool]);

  if (!desktop) return <MobileTasksSheet sessionId={sessionId} background={background} onOpenTool={onOpenTool} />;
  if (desktopHostActive) return null;
  return <StandaloneDesktopTasksDialog sessionId={sessionId} background={background} onLocate={onLocate} onOpenTool={onOpenTool} />;
}
