// Structured browser-history state machine for MobileTasksSheet.

import { type MutableRefObject, useCallback, useEffect, useRef, useState } from "react";
import { acknowledgeTask, taskHash, taskKeyFromHash, type TaskSummary } from "../backgroundTasks";
import { closeBackgroundTasks, inspectorState, openBackgroundTask, OPEN_BACKGROUND_TASKS_EVENT, selectBackgroundTask, useInspectorState } from "../backgroundTaskInspectorState";
import { getActiveSessionId, subscribeActiveSession } from "../controlPlane";
import { requestPickSession } from "../sessionPickRequest";
import type { BackgroundTasks } from "../useBackgroundTasks";
import { clearConsumedDeepLinks, consumeDeepLink, isDeepLinkConsumed } from "./taskEvents";
import { defaultReaderMode, type TaskReaderContentMode } from "./TaskResultReader";

export type MobileLevel = "peek" | "reader" | "info";

let nextMobileHistoryOpenId = 1;
let activeChildSwitchToken = 0;
const pendingCloseUnwindSessions = new Set<string>();

interface MobileTasksHistoryEntry {
  openId: number;
  sessionId: string;
  depth: number;
  level: MobileLevel;
  key: string | null;
}

type PendingPopAction =
  | { kind: "peek" }
  | { kind: "mode"; mode: TaskReaderContentMode }
  | { kind: "sibling"; key: string }
  | null;

interface HistoryRefs {
  openId: MutableRefObject<number>;
  depth: MutableRefObject<number>;
  initialized: MutableRefObject<{ revision: number; key: string | null; resolved: boolean } | null>;
  lastContentMode: MutableRefObject<{ key: string; mode: TaskReaderContentMode } | null>;
  pendingPop: MutableRefObject<PendingPopAction>;
}

function makeHistoryState(entry: MobileTasksHistoryEntry): { cowboyTasks: MobileTasksHistoryEntry } {
  return { cowboyTasks: entry };
}

function readHistoryState(state: unknown): MobileTasksHistoryEntry | null {
  if (!state || typeof state !== "object") return null;
  const raw = (state as { cowboyTasks?: unknown }).cowboyTasks;
  if (!raw || typeof raw !== "object") return null;
  const e = raw as Partial<MobileTasksHistoryEntry>;
  if (typeof e.openId !== "number" || typeof e.sessionId !== "string" || typeof e.depth !== "number") return null;
  if (e.level !== "peek" && e.level !== "reader" && e.level !== "info") return null;
  return { openId: e.openId, sessionId: e.sessionId, depth: e.depth, level: e.level, key: typeof e.key === "string" ? e.key : null };
}

function applyMobilePopState({
  event,
  sessionId,
  byKey,
  refs,
  setLevel,
  setReaderMode,
}: {
  event: PopStateEvent;
  sessionId: string;
  byKey: ReadonlyMap<string, TaskSummary>;
  refs: HistoryRefs;
  setLevel: (level: MobileLevel) => void;
  setReaderMode: (mode: TaskReaderContentMode) => void;
}): void {
  const dest = readHistoryState(event.state);
  if (!dest || dest.sessionId !== sessionId || dest.openId !== refs.openId.current) {
    refs.openId.current = 0;
    refs.depth.current = 0;
    refs.pendingPop.current = null;
    closeBackgroundTasks();
    return;
  }
  refs.depth.current = dest.depth;
  const pending = refs.pendingPop.current;
  refs.pendingPop.current = null;
  if (pending?.kind === "peek") {
    if (dest.level !== "peek") {
      globalThis.history?.replaceState(makeHistoryState({ openId: refs.openId.current, sessionId, depth: 1, level: "peek", key: null }), "", "#tasks");
    }
    refs.initialized.current = { revision: inspectorState().revision, key: null, resolved: false };
    selectBackgroundTask(null);
    setLevel("peek");
    return;
  }
  if (pending?.kind === "sibling") {
    const nextTask = byKey.get(pending.key);
    if (nextTask) {
      const mode = defaultReaderMode(nextTask, false);
      refs.initialized.current = { revision: inspectorState().revision, key: nextTask.key, resolved: true };
      selectBackgroundTask(nextTask.key);
      setLevel("reader");
      setReaderMode(mode);
      refs.lastContentMode.current = { key: nextTask.key, mode };
      if (nextTask.attention === "failed_unseen") acknowledgeTask(sessionId, nextTask);
      globalThis.history?.replaceState(makeHistoryState({ openId: refs.openId.current, sessionId, depth: dest.depth, level: "reader", key: nextTask.key }), "", taskHash(nextTask.key));
      return;
    }
  }
  if (dest.level === "peek" || !dest.key) {
    refs.initialized.current = { revision: inspectorState().revision, key: null, resolved: false };
    selectBackgroundTask(null);
    setLevel("peek");
    return;
  }
  const target = byKey.get(dest.key) ?? null;
  refs.initialized.current = { revision: inspectorState().revision, key: dest.key, resolved: Boolean(target) };
  selectBackgroundTask(dest.key);
  if (dest.level === "info") { setLevel("info"); return; }
  const mode: TaskReaderContentMode = pending?.kind === "mode"
    ? pending.mode
    : refs.lastContentMode.current?.key === dest.key
    ? refs.lastContentMode.current.mode
    : target
    ? defaultReaderMode(target, false)
    : "result";
  setLevel("reader");
  setReaderMode(mode);
  refs.lastContentMode.current = { key: dest.key, mode };
}

/** Unwind all history entries belonging to `retiringOpenId` before switching to
 *  `childId`. Switches only after a matching popstate exits `retiringOpenId`,
 *  and expires harmlessly if newer navigation claims ownership. */
function unwindCloseHistory(sessionId: string, retiringOpenId: number, steps: number): void {
  if (steps <= 0) return;
  pendingCloseUnwindSessions.add(sessionId);
  const onClosePop = (event: PopStateEvent): void => {
    const dest = readHistoryState(event.state);
    if (dest && dest.openId === retiringOpenId) return;
    globalThis.removeEventListener("popstate", onClosePop);
    pendingCloseUnwindSessions.delete(sessionId);
    const landedHash = globalThis.location?.hash ?? "";
    if (!taskKeyFromHash(landedHash)) clearConsumedDeepLinks(sessionId);
  };
  globalThis.addEventListener("popstate", onClosePop);
  globalThis.history?.go(-steps);
}

function unwindAndSwitchSession(
  parentSessionId: string,
  steps: number,
  retiringOpenId: number,
  childId: string,
): void {
  const token = ++activeChildSwitchToken;
  let unsubscribeSession: (() => void) | null = null;
  let onPopRef: ((event: PopStateEvent) => void) | null = null;
  const cleanup = (): void => {
    if (onPopRef) { globalThis.removeEventListener("popstate", onPopRef); onPopRef = null; }
    if (unsubscribeSession) { unsubscribeSession(); unsubscribeSession = null; }
  };
  const finish = (): void => {
    cleanup();
    if (activeChildSwitchToken !== token) return;
    activeChildSwitchToken += 1;
    if (getActiveSessionId() !== parentSessionId) return;
    const currentHash = globalThis.location?.hash ?? "";
    if (taskKeyFromHash(currentHash) || currentHash === "#tasks") {
      globalThis.history?.replaceState(globalThis.history.state, "", `${globalThis.location.pathname}${globalThis.location.search}`);
    }
    requestPickSession(childId);
  };
  if (steps <= 0) { finish(); return; }
  unsubscribeSession = subscribeActiveSession(() => {
    if (getActiveSessionId() !== parentSessionId) {
      if (activeChildSwitchToken === token) activeChildSwitchToken += 1;
      cleanup();
    }
  });
  const onPop = (event: PopStateEvent): void => {
    if (activeChildSwitchToken !== token || getActiveSessionId() !== parentSessionId) {
      cleanup();
      return;
    }
    const dest = readHistoryState(event.state);
    if (dest && dest.openId === retiringOpenId) return;
    finish();
  };
  onPopRef = onPop;
  globalThis.addEventListener("popstate", onPop);
  globalThis.history?.go(-steps);
}

function useMobileDeepLinkAndCommand(
  sessionId: string,
  byKey: ReadonlyMap<string, TaskSummary>,
  open: boolean,
): void {
  const [hashEpoch, setHashEpoch] = useState(0);
  useEffect(() => {
    const onHashChange = (): void => {
      const rawHash = globalThis.location?.hash ?? "";
      // Ignore hash changes caused by internal Tasks history entries or unwinding
      // back to an already-consumed initial fragment; only clear when external
      // navigation moves away from task hashes entirely.
      if (!rawHash || (!taskKeyFromHash(rawHash) && rawHash !== "#tasks")) {
        clearConsumedDeepLinks(sessionId);
      }
      setHashEpoch((n) => n + 1);
    };
    globalThis.addEventListener("hashchange", onHashChange);
    return () => globalThis.removeEventListener("hashchange", onHashChange);
  }, [sessionId]);
  useEffect(() => {
    const rawHash = globalThis.location?.hash ?? "";
    const key = taskKeyFromHash(rawHash);
    if (!key) { if (!open && !pendingCloseUnwindSessions.has(sessionId)) clearConsumedDeepLinks(sessionId); return; }
    if (open || inspectorState().sessionId === sessionId || isDeepLinkConsumed(sessionId, rawHash) || !byKey.has(key)) return;
    consumeDeepLink(sessionId, rawHash);
    openBackgroundTask(sessionId, key, "link");
  }, [sessionId, byKey, open, hashEpoch]);
  useEffect(() => {
    const onOpen = (event: Event): void => {
      const detail = (event as CustomEvent<{ sessionId?: string }>).detail;
      if (detail?.sessionId && detail.sessionId !== sessionId) return;
      openBackgroundTask(sessionId, null, "command");
    };
    globalThis.addEventListener(OPEN_BACKGROUND_TASKS_EVENT, onOpen);
    return () => globalThis.removeEventListener(OPEN_BACKGROUND_TASKS_EVENT, onOpen);
  }, [sessionId]);
}

export function useMobileTasksHistory({ sessionId, background, visibleTasks }: { sessionId: string; background: BackgroundTasks; visibleTasks: readonly TaskSummary[] }) {
  const state = useInspectorState();
  const open = state.sessionId === sessionId;
  const [level, setLevel] = useState<MobileLevel>("peek");
  const [readerMode, setReaderMode] = useState<TaskReaderContentMode>("result");
  const openIdRef = useRef(0);
  const depthRef = useRef(0);
  const initializedRef = useRef<{ revision: number; key: string | null; resolved: boolean } | null>(null);
  const lastContentModeRef = useRef<{ key: string; mode: TaskReaderContentMode } | null>(null);
  const pendingPopRef = useRef<PendingPopAction>(null);
  const selected = state.key ? background.byKey.get(state.key) ?? null : null;
  useMobileDeepLinkAndCommand(sessionId, background.byKey, open);

  useEffect(() => {
    if (!open) { openIdRef.current = 0; depthRef.current = 0; initializedRef.current = null; pendingPopRef.current = null; return; }
    activeChildSwitchToken += 1;
    pendingCloseUnwindSessions.delete(sessionId);
    const target = state.key ? background.byKey.get(state.key) ?? null : null;
    const prev = initializedRef.current;
    if (prev && prev.revision === state.revision && prev.key === state.key && (prev.resolved || !target)) return;
    initializedRef.current = { revision: state.revision, key: state.key, resolved: Boolean(target) };
    if (target) {
      const mode = defaultReaderMode(target, false);
      setLevel("reader");
      setReaderMode(mode);
      lastContentModeRef.current = { key: target.key, mode };
      if (state.origin !== "link" && target.attention === "failed_unseen") acknowledgeTask(sessionId, target);
    } else {
      setLevel("peek");
    }
    if (openIdRef.current === 0) openIdRef.current = nextMobileHistoryOpenId++;
    const entry: MobileTasksHistoryEntry = { openId: openIdRef.current, sessionId, depth: 1, level: target ? "reader" : "peek", key: target?.key ?? null };
    if (depthRef.current === 0) {
      globalThis.history?.pushState(makeHistoryState(entry), "", target ? taskHash(target.key) : "#tasks");
      depthRef.current = 1;
    } else if (depthRef.current === 1 && target) {
      globalThis.history?.replaceState(makeHistoryState(entry), "", taskHash(target.key));
    }
  }, [open, state.revision, state.key, state.origin, sessionId, background.byKey]);

  useEffect(() => {
    if (!open) return undefined;
    const refs: HistoryRefs = { openId: openIdRef, depth: depthRef, initialized: initializedRef, lastContentMode: lastContentModeRef, pendingPop: pendingPopRef };
    const onPop = (event: PopStateEvent): void => applyMobilePopState({ event, sessionId, byKey: background.byKey, refs, setLevel, setReaderMode });
    globalThis.addEventListener("popstate", onPop);
    return () => globalThis.removeEventListener("popstate", onPop);
  }, [open, sessionId, background.byKey]);

  const close = useCallback((): void => {
    activeChildSwitchToken += 1;
    const retiringOpenId = openIdRef.current;
    const steps = depthRef.current;
    openIdRef.current = 0;
    depthRef.current = 0;
    pendingPopRef.current = null;
    const rawHash = globalThis.location?.hash ?? "";
    if (taskKeyFromHash(rawHash)) consumeDeepLink(sessionId, rawHash);
    closeBackgroundTasks();
    unwindCloseHistory(sessionId, retiringOpenId, steps);
  }, [sessionId]);

  const openWorkerFromPeek = useCallback((key: string): void => {
    const target = background.byKey.get(key);
    if (!target) return;
    const mode = defaultReaderMode(target, false);
    initializedRef.current = { revision: inspectorState().revision, key, resolved: true };
    selectBackgroundTask(key);
    setLevel("reader");
    setReaderMode(mode);
    lastContentModeRef.current = { key, mode };
    if (target.attention === "failed_unseen") acknowledgeTask(sessionId, target);
    depthRef.current += 1;
    globalThis.history?.pushState(makeHistoryState({ openId: openIdRef.current, sessionId, depth: depthRef.current, level: "reader", key }), "", taskHash(key));
  }, [background.byKey, sessionId]);

  const switchSibling = useCallback((key: string): void => {
    const target = background.byKey.get(key);
    if (!target) return;
    if (level === "info" && depthRef.current > 1) { pendingPopRef.current = { kind: "sibling", key }; globalThis.history?.back(); return; }
    const mode = defaultReaderMode(target, false);
    initializedRef.current = { revision: inspectorState().revision, key, resolved: true };
    selectBackgroundTask(key);
    setLevel("reader");
    setReaderMode(mode);
    lastContentModeRef.current = { key, mode };
    if (target.attention === "failed_unseen") acknowledgeTask(sessionId, target);
    globalThis.history?.replaceState(makeHistoryState({ openId: openIdRef.current, sessionId, depth: Math.max(1, depthRef.current), level: "reader", key }), "", taskHash(key));
  }, [background.byKey, level, sessionId]);

  const stepSibling = useCallback((delta: -1 | 1): void => {
    if (visibleTasks.length <= 1 || !selected) return;
    const index = visibleTasks.findIndex((item) => item.key === selected.key);
    const next = visibleTasks[(index + delta + visibleTasks.length) % visibleTasks.length];
    if (next) switchSibling(next.key);
  }, [selected, switchSibling, visibleTasks]);

  const toggleInfo = useCallback((): void => {
    if (!selected) return;
    if (level === "info") { if (depthRef.current > 1) globalThis.history?.back(); else setLevel("reader"); return; }
    setLevel("info");
    depthRef.current += 1;
    globalThis.history?.pushState(makeHistoryState({ openId: openIdRef.current, sessionId, depth: depthRef.current, level: "info", key: selected.key }), "", taskHash(selected.key));
  }, [level, selected, sessionId]);

  const setContentMode = useCallback((next: TaskReaderContentMode): void => {
    if (selected) lastContentModeRef.current = { key: selected.key, mode: next };
    if (level === "info" && depthRef.current > 1) { pendingPopRef.current = { kind: "mode", mode: next }; globalThis.history?.back(); return; }
    setLevel("reader");
    setReaderMode(next);
  }, [level, selected]);

  const showPeek = useCallback((): void => {
    if (depthRef.current > 1) { pendingPopRef.current = { kind: "peek" }; globalThis.history?.go(-(depthRef.current - 1)); return; }
    initializedRef.current = { revision: inspectorState().revision, key: null, resolved: false };
    selectBackgroundTask(null);
    setLevel("peek");
    globalThis.history?.replaceState(makeHistoryState({ openId: openIdRef.current, sessionId, depth: 1, level: "peek", key: null }), "", "#tasks");
  }, [sessionId]);

  const openChildSession = useCallback((childId: string): void => {
    const retiringOpenId = openIdRef.current;
    const steps = depthRef.current;
    openIdRef.current = 0;
    depthRef.current = 0;
    pendingPopRef.current = null;
    const rawHash = globalThis.location?.hash ?? "";
    if (taskKeyFromHash(rawHash)) consumeDeepLink(sessionId, rawHash);
    closeBackgroundTasks();
    unwindAndSwitchSession(sessionId, steps, retiringOpenId, childId);
  }, [sessionId]);

  const currentOpenId = useCallback((): number => openIdRef.current, []);
  const isStillOwner = useCallback((ownerOpenId: number, taskKey: string): boolean => {
    const live = inspectorState();
    return ownerOpenId > 0 && openIdRef.current === ownerOpenId && live.sessionId === sessionId && live.key === taskKey;
  }, [sessionId]);

  return { open, selected, level, readerMode, setContentMode, close, openWorkerFromPeek, switchSibling, stepSibling, toggleInfo, showPeek, openChildSession, currentOpenId, isStillOwner };
}
