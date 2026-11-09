// One runtime per app identity (docs/widget-apps.md §4.3–§5.3). The inline
// row and the inspector are views of the same runtime and tree, so moving
// between them keeps the app's state; the most recent view is primary and the
// others show a placeholder. At most four runtimes live per page, render
// credit is paced by the work each batch commits, and a hidden page suspends
// every app.

import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { startAppRuntime, type AppRuntime } from "./appSandbox";
import { AppBudgetError, applyAppOps, emptyAppTree, type AppTreeState } from "./appTree";

const MAX_LIVE = 4;
const UNITS_PER_FRAME = 1500;
const MIN_CREDIT_INTERVAL_MS = 33;
const RELEASE_GRACE_MS = 1500;

export type AppStart = { code: string; props: unknown; context: unknown };

type Entry = {
  runtime: AppRuntime | null;
  tree: AppTreeState;
  stopped: string | null;
  views: symbol[];
  dirty: boolean;
  onSend: ((text: string) => void) | undefined;
  listeners: Set<() => void>;
  release: ReturnType<typeof setTimeout> | null;
  lastCredit: number;
  start: AppStart;
};

const entries = new Map<string, Entry>();
const version = { value: 0 };
const pageListeners = new Set<() => void>();

function changed(entry: Entry): void {
  version.value += 1;
  for (const listener of entry.listeners) listener();
  for (const listener of pageListeners) listener();
}

function credit(entry: Entry, units: number): void {
  // Return credit after the frames this commit's work deserves.
  let frames = Math.max(1, Math.ceil(units / UNITS_PER_FRAME));
  const tick = (): void => {
    frames -= 1;
    if (frames > 0) return void requestAnimationFrame(tick);
    const wait = entry.lastCredit + MIN_CREDIT_INTERVAL_MS - performance.now();
    const grant = (): void => {
      entry.lastCredit = performance.now();
      entry.runtime?.credit();
    };
    if (wait > 0) setTimeout(grant, wait);
    else grant();
  };
  requestAnimationFrame(tick);
}

function launch(entry: Entry): void {
  entry.tree = emptyAppTree();
  entry.stopped = null;
  entry.dirty = false;
  const runtime = startAppRuntime(entry.start, {
    onOps: (json) => {
      try {
        const before = entry.tree.units;
        entry.tree = applyAppOps(entry.tree, json);
        changed(entry);
        credit(entry, Math.abs(entry.tree.units - before) + 1);
      } catch (error) {
        stopEntry(
          entry,
          `This app stopped: ${error instanceof AppBudgetError ? error.message : "it sent a bad update"}.`,
        );
      }
    },
    onSend: (text) => entry.onSend?.(text),
    onStopped: (reason) => {
      if (reason === "closed") return;
      entry.runtime = null;
      entry.stopped = reason;
      changed(entry);
    },
  });
  entry.runtime = runtime;
  if (typeof document !== "undefined" && document.hidden) runtime.suspend();
}

function stopEntry(entry: Entry, reason: string): void {
  entry.runtime?.terminate();
  entry.runtime = null;
  entry.stopped = reason;
  changed(entry);
}

const live = (): number => [...entries.values()].filter((entry) => entry.runtime !== null).length;

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => {
    for (const entry of entries.values()) {
      if (document.hidden) entry.runtime?.suspend();
      else entry.runtime?.resume();
    }
  });
}

export type AppView = {
  tree: AppTreeState;
  stopped: string | null;
  primary: boolean;
  /** The page already runs the maximum number of apps. */
  waiting: boolean;
  dirty: boolean;
  dispatch: (id: number, name: string, payload: unknown) => void;
  reload: () => void;
};

/** A view of the runtime for `key`, starting it if this is the first view. */
export function useAppRuntime(
  key: string,
  start: AppStart,
  onSend: ((text: string) => void) | undefined,
): AppView {
  const view = useMemo(() => Symbol(key), [key]);
  const [waiting, setWaiting] = useState(false);
  useEffect(() => {
    let entry = entries.get(key);
    if (!entry) {
      if (live() >= MAX_LIVE) {
        setWaiting(true);
        const retry = (): void => {
          if (live() < MAX_LIVE) {
            pageListeners.delete(retry);
            setWaiting(false);
          }
        };
        pageListeners.add(retry);
        return () => void pageListeners.delete(retry);
      }
      entry = {
        runtime: null,
        tree: emptyAppTree(),
        stopped: null,
        views: [],
        dirty: false,
        onSend: undefined,
        listeners: new Set(),
        release: null,
        lastCredit: 0,
        start,
      };
      entries.set(key, entry);
      launch(entry);
    }
    if (entry.release) clearTimeout(entry.release);
    entry.release = null;
    entry.views.push(view);
    changed(entry);
    const owned = entry;
    return () => {
      owned.views = owned.views.filter((other) => other !== view);
      changed(owned);
      if (owned.views.length === 0) {
        owned.release = setTimeout(() => {
          owned.runtime?.terminate();
          entries.delete(key);
          changed(owned);
        }, RELEASE_GRACE_MS);
      }
    };
    // `start` only seeds a new runtime; later props reach it below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, view, waiting]);

  const subscribe = (listener: () => void): (() => void) => {
    pageListeners.add(listener);
    return () => void pageListeners.delete(listener);
  };
  useSyncExternalStore(subscribe, () => version.value);
  const entry = entries.get(key);
  const primary = entry ? entry.views.at(-1) === view : false;
  if (entry && primary) entry.onSend = onSend;
  const propsJson = JSON.stringify(start.props ?? {});
  const contextJson = JSON.stringify(start.context ?? {});
  useEffect(() => {
    if (!entry || !primary) return;
    entry.start = { ...entry.start, props: JSON.parse(propsJson) as unknown };
    entry.runtime?.props(JSON.parse(propsJson) as unknown);
  }, [entry, primary, propsJson]);
  useEffect(() => {
    if (!entry || !primary) return;
    entry.runtime?.context(JSON.parse(contextJson) as unknown);
  }, [entry, primary, contextJson]);
  return {
    tree: entry?.tree ?? emptyAppTree(),
    stopped: entry?.stopped ?? null,
    primary,
    waiting,
    dirty: entry?.dirty ?? false,
    dispatch: (id, name, payload) => {
      if (!entry?.runtime) return;
      if (name === "onChange") entry.dirty = true;
      entry.runtime.event(id, name, payload);
    },
    reload: () => {
      if (!entry) return;
      entry.runtime?.terminate();
      launch(entry);
      changed(entry);
    },
  };
}

/** Stops the runtime of `key` with a person-readable reason (renderer failure). */
export function stopAppRuntime(key: string, reason: string): void {
  const entry = entries.get(key);
  if (entry) stopEntry(entry, reason);
}

/** Whether the person typed into the running app of `key`. */
export function appRuntimeDirty(key: string): boolean {
  return entries.get(key)?.dirty ?? false;
}
