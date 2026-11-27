// State, deep-link, inspector sync, and scoped command hooks for DesktopTasksView.

import { type RefObject, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { acknowledgeTask, taskKeyFromHash, type TaskSummary } from "../backgroundTasks";
import { inspectorState, openBackgroundTask, selectBackgroundTask, useInspectorState } from "../backgroundTaskInspectorState";
import { type DesktopCommand, useDesktopCommandActions } from "../desktop/commands/DesktopCommandProvider";
import { shortcutKey } from "../desktop/commands/imeShortcut";
import { requestDesktopWorkView } from "../desktop/review/desktopWorkView";
import { type BackgroundTasks, useBackgroundTasks } from "../useBackgroundTasks";
import { clearConsumedDeepLinks, clearSessionTasksAvailability, consumeDeepLink, isDeepLinkConsumed, publishSessionTasksAvailability, registerDesktopTasksHost } from "./taskEvents";
import type { TaskReaderContentMode, TaskReaderViewMode } from "./TaskResultReader";
import { buildTaskForest, flattenTaskSections, taskTabSummary } from "./taskTree";

const TASKS_REGIONS = ["tasks.roster", "tasks.reader"] as const;

export interface DesktopModeState {
  key: string;
  mode: TaskReaderViewMode;
  prev: TaskReaderContentMode;
}

export interface DesktopTasksCommandActions {
  active: boolean;
  selected: TaskSummary | null;
  hasConversation: boolean;
  effectiveMode: TaskReaderViewMode;
  hideEnded: boolean;
  stepTask: (delta: number) => void;
  toggleInfo: () => void;
  toggleTranscript: () => void;
  quote: () => void;
  openChild: () => void;
  locate: () => void;
  stop: () => void;
  escape: () => void;
}

/** Register scoped Tasks commands with DesktopCommandProvider so shortcuts
 *  integrate cleanly with workspace focus, palette, and region scoping. */
export function useDesktopTasksCommands(actions: DesktopTasksCommandActions): void {
  const { register } = useDesktopCommandActions();
  const latest = useRef(actions);
  latest.current = actions;
  const commands = useMemo((): DesktopCommand[] => {
    const a = (): DesktopTasksCommandActions => latest.current;
    const ready = (): boolean => a().active;
    const scoped = (id: string, shortcut: string, title: string, run: () => void, when?: () => boolean): DesktopCommand => ({
      id,
      title,
      group: "Tasks",
      shortcut,
      contexts: ["conversation"],
      regions: [...TASKS_REGIONS],
      when: () => ready() && (when ? when() : true),
      run,
    });
    return [
      scoped("tasks.previous", "[", "Previous task", () => a().stepTask(-1), () => a().selected !== null),
      scoped("tasks.next", "]", "Next task", () => a().stepTask(1), () => a().selected !== null),
      scoped("tasks.info", "I", "Toggle task info", () => a().toggleInfo(), () => a().selected !== null),
      scoped("tasks.transcript", "T", "Toggle Result / Transcript", () => a().toggleTranscript(), () => a().selected !== null && a().hasConversation),
      scoped("tasks.quote", "S", "Quote task to Prompt", () => a().quote(), () => a().selected !== null),
      scoped("tasks.openSession", "O", "Open Call as session", () => a().openChild(), () => a().selected?.source === "call" && Boolean(a().selected?.call)),
      scoped("tasks.locate", "Shift+L", "Locate task in Conversation", () => a().locate(), () => a().selected !== null && a().selected?.anchorSeq !== null),
      scoped("tasks.stop", "X", "Stop selected task", () => a().stop(), () => Boolean(a().selected?.canStop)),
      scoped("tasks.escape", "Escape", "Close info or clear filter", () => a().escape(), () => a().effectiveMode === "info" || a().hideEnded),
    ];
  }, []);
  useEffect(() => {
    const releases = commands.map((command) => register(command));
    return () => releases.forEach((release) => release());
  }, [commands, register]);
}

export function useDesktopTasksPaneKeyHandler({
  active,
  rootRef,
  byKey,
  selected,
  stepTask,
  chooseTask,
  handleAnswerApproval,
  focusReader,
}: {
  active: boolean;
  rootRef: RefObject<HTMLDivElement | null>;
  byKey: ReadonlyMap<string, TaskSummary>;
  selected: TaskSummary | null;
  stepTask: (delta: number) => void;
  chooseTask: (key: string) => void;
  handleAnswerApproval: (task: TaskSummary) => void;
  focusReader: () => void;
}): (event: React.KeyboardEvent<HTMLDivElement>) => void {
  return useCallback((event: React.KeyboardEvent<HTMLDivElement>): void => {
    if (!active || event.metaKey || event.ctrlKey || event.altKey) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest("input, textarea, select, [contenteditable='true'], .cm-editor, [role='dialog']")) return;
    const key = shortcutKey(event.nativeEvent);
    if (!key) return;
    const inRoster = Boolean(target?.closest("[data-desktop-region='tasks.roster']"));
    if (key === "j" || key === "ArrowDown" || key === "k" || key === "ArrowUp") {
      const delta = key === "j" || key === "ArrowDown" ? 1 : -1;
      if (inRoster) { event.preventDefault(); event.stopPropagation(); stepTask(delta); return; }
      const scroller = rootRef.current?.querySelector<HTMLElement>("[data-task-reader-scroll], [data-task-transcript-host]:not([hidden]) [data-desktop-transcript-scroller]");
      if (scroller) { event.preventDefault(); event.stopPropagation(); scroller.scrollBy({ top: delta * 72 }); }
      return;
    }
    if (key === "Enter" && inRoster) {
      const optionEl = target?.closest<HTMLElement>("[data-task-option]");
      const nonOptionControl = !optionEl && Boolean(target?.closest("button, a, [role='button'], [role='switch'], [role='checkbox'], [data-task-fold], [data-task-hide-ended]"));
      if (nonOptionControl) return;
      const targetTask = (optionEl?.dataset["taskOption"] ? byKey.get(optionEl.dataset["taskOption"]) : null) ?? selected;
      if (!targetTask) return;
      event.preventDefault();
      event.stopPropagation();
      if (targetTask.source === "approval") handleAnswerApproval(targetTask);
      else { chooseTask(targetTask.key); requestAnimationFrame(focusReader); }
    }
  }, [active, byKey, chooseTask, focusReader, handleAnswerApproval, rootRef, selected, stepTask]);
}

export function useDesktopTasksPaneWidth(rootRef: RefObject<HTMLDivElement | null>, active: boolean): number {
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

export function useDesktopTasksController(sessionId: string, active: boolean, providedBackground?: BackgroundTasks) {
  const liveBackground = useBackgroundTasks(sessionId);
  const background = providedBackground ?? liveBackground;
  const inspector = useInspectorState();
  const [hideEnded, setHideEnded] = useState(false);
  const [collapsedKeys, setCollapsedKeys] = useState<ReadonlySet<string>>(() => new Set());
  const [compactShowRoster, setCompactShowRoster] = useState(false);
  const [readerMode, setReaderMode] = useState<DesktopModeState | null>(null);
  const [localKey, setLocalKey] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const handledInspectorRef = useRef<{ revision: number; key: string | null; resolved: boolean } | null>(null);
  const [hashEpoch, setHashEpoch] = useState(0);

  useEffect(() => registerDesktopTasksHost(sessionId), [sessionId]);
  useEffect(() => {
    const onHashChange = (): void => {
      const rawHash = globalThis.location?.hash ?? "";
      if (!taskKeyFromHash(rawHash)) clearConsumedDeepLinks(sessionId);
      setHashEpoch((n) => n + 1);
    };
    globalThis.addEventListener("hashchange", onHashChange);
    return () => globalThis.removeEventListener("hashchange", onHashChange);
  }, [sessionId]);

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
    const preferred = localKey ?? (inspector.sessionId === sessionId ? inspector.key : null);
    if (preferred && background.byKey.has(preferred)) return preferred;
    return visibleTasks[0]?.key ?? background.tasks[0]?.key ?? null;
  }, [inspector.sessionId, inspector.key, localKey, sessionId, background.byKey, visibleTasks, background.tasks]);
  const selected = selectedKey ? background.byKey.get(selectedKey) ?? null : null;

  useEffect(() => {
    const rawHash = globalThis.location?.hash ?? "";
    const key = taskKeyFromHash(rawHash);
    if (!key) { clearConsumedDeepLinks(sessionId); return; }
    if (isDeepLinkConsumed(sessionId, rawHash) || !background.byKey.has(key)) return;
    consumeDeepLink(sessionId, rawHash);
    globalThis.history?.replaceState(globalThis.history.state, "", `${globalThis.location.pathname}${globalThis.location.search}`);
    setLocalKey(key);
    setCompactShowRoster(false);
    openBackgroundTask(sessionId, key, "link");
    requestDesktopWorkView("tasks");
  }, [sessionId, background.byKey, hashEpoch]);

  useEffect(() => {
    if (inspector.sessionId !== sessionId) return;
    const target = inspector.key ? background.byKey.get(inspector.key) ?? null : null;
    const prev = handledInspectorRef.current;
    if (prev && prev.revision === inspector.revision && prev.key === inspector.key && (prev.resolved || !target)) return;
    handledInspectorRef.current = { revision: inspector.revision, key: inspector.key, resolved: Boolean(target) };
    if (target) {
      setLocalKey(target.key);
      setCompactShowRoster(false);
      setReaderMode(null);
      if (inspector.origin !== "link" && target.attention === "failed_unseen") acknowledgeTask(sessionId, target);
    } else if (inspector.key === null && background.tasks.length > 1) {
      setCompactShowRoster(true);
    }
  }, [inspector.revision, inspector.sessionId, inspector.key, inspector.origin, sessionId, background.byKey, background.tasks.length]);

  const chooseTask = useCallback((key: string): void => {
    setLocalKey(key);
    setCompactShowRoster(false);
    setReaderMode(null);
    if (inspectorState().sessionId === sessionId) selectBackgroundTask(key);
    const picked = background.byKey.get(key);
    if (picked?.attention === "failed_unseen") acknowledgeTask(sessionId, picked);
  }, [background.byKey, sessionId]);

  const stepTask = useCallback((delta: number): void => {
    if (visibleTasks.length === 0) return;
    const index = selected ? visibleTasks.findIndex((item) => item.key === selected.key) : -1;
    const next = visibleTasks[(index === -1 ? 0 : index + delta + visibleTasks.length) % visibleTasks.length];
    if (next) chooseTask(next.key);
  }, [visibleTasks, selected, chooseTask]);

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
