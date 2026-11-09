// One runtime per app (docs/widget-apps.md §4.3–§6). The inline row and the
// inspector are views of the same runtime and tree, so moving between them
// keeps the app's state; the most recent view is primary and the others show
// a placeholder. The registry also owns which version of the code runs: a
// newer version replaces an untouched app at once, while an app the person
// typed into keeps its code and props until they choose. At most four
// runtimes live per page; render batches commit under a per-frame unit
// budget shared by every app (credits wait with them), each session view's
// apps share one unit budget, and a hidden page suspends every app.

import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { startAppRuntime, type AppRuntime } from "./appSandbox";
import { AppBudgetError, applyAppOps, emptyAppTree, type AppTreeState } from "./appTree";

const MAX_LIVE = 4;
const UNITS_PER_FRAME = 1500;
const SESSION_UNITS = 12_000;
const MIN_CREDIT_INTERVAL_MS = 33;
const RELEASE_GRACE_MS = 1500;

export type AppStart = { code: string; props: unknown; context: unknown };

type Entry = {
  key: string;
  /** Apps of one session view share the session unit budget. */
  scope: string;
  runtime: AppRuntime | null;
  tree: AppTreeState;
  stopped: string | null;
  views: symbol[];
  /** The person typed into the app or it asked to send: its state matters. */
  dirty: boolean;
  /** A newer code version the person chose not to load yet. */
  kept: string | null;
  onSend: ((text: string, code: string) => void) | undefined;
  release: ReturnType<typeof setTimeout> | null;
  /** When the app last lost its final view (eviction order). */
  hiddenAt: number;
  lastCredit: number;
  /** The running code with its props and context. */
  start: AppStart;
  /** Render batches waiting for a frame with budget left. */
  batches: string[];
};

const entries = new Map<string, Entry>();
const version = { value: 0 };
const pageListeners = new Set<() => void>();

function changed(): void {
  version.value += 1;
  for (const listener of pageListeners) listener();
}

// ---- Frame budget ------------------------------------------------------------

const drawing: Entry[] = [];
let frameRequested = false;
let debt = 0;

function requestFrame(): void {
  if (frameRequested) return;
  frameRequested = true;
  requestAnimationFrame(frame);
}

function grantCredit(entry: Entry): void {
  const runtime = entry.runtime;
  const wait = entry.lastCredit + MIN_CREDIT_INTERVAL_MS - performance.now();
  const grant = (): void => {
    entry.lastCredit = performance.now();
    if (entry.runtime === runtime) runtime?.credit();
  };
  if (wait > 0) setTimeout(grant, wait);
  else grant();
}

const scopeUnits = (scope: string, except: Entry): number =>
  [...entries.values()]
    .filter((other) => other !== except && other.scope === scope && other.runtime !== null)
    .reduce((sum, other) => sum + other.tree.units, 0);

/** Applies an app's next batch, or stops the app if the batch breaks a budget. */
function prepare(entry: Entry, json: string): AppTreeState | null {
  try {
    const next = applyAppOps(entry.tree, json);
    if (scopeUnits(entry.scope, entry) + next.units > SESSION_UNITS) {
      throw new AppBudgetError("the apps in this session drew too much together");
    }
    return next;
  } catch (error) {
    stopEntry(
      entry,
      `This app stopped: ${error instanceof AppBudgetError ? error.message : "it sent a bad update"}.`,
    );
    return null;
  }
}

/** Commits waiting batches, round-robin across apps, until this frame's
 *  units are spent. A batch larger than one frame commits alone and the
 *  frames it overran stay empty, so every credit returns that much later. */
function frame(): void {
  frameRequested = false;
  if (debt > 0) {
    debt = Math.max(0, debt - UNITS_PER_FRAME);
    if (drawing.length > 0 || debt > 0) requestFrame();
    return;
  }
  let budget = UNITS_PER_FRAME;
  let committed = false;
  while (drawing.length > 0 && budget > 0) {
    const entry = drawing[0]!;
    const json = entry.batches[0];
    const next = json === undefined || !entry.runtime ? null : prepare(entry, json);
    if (!next) {
      drawing.shift();
      continue;
    }
    if (committed && next.charged > budget) break;
    drawing.shift();
    entry.batches.shift();
    entry.tree = next;
    committed = true;
    budget -= next.charged;
    grantCredit(entry);
    if (entry.batches.length > 0) drawing.push(entry);
  }
  if (budget < 0) debt = -budget;
  if (committed) changed();
  if (drawing.length > 0 || debt > 0) requestFrame();
}

// ---- Lifecycle ---------------------------------------------------------------

function launch(entry: Entry): void {
  entry.tree = emptyAppTree();
  entry.batches = [];
  entry.stopped = null;
  entry.dirty = false;
  const runtime = startAppRuntime(entry.start, {
    onOps: (json) => {
      if (entry.runtime !== runtime) return;
      entry.batches.push(json);
      if (entry.batches.length === 1) drawing.push(entry);
      requestFrame();
    },
    onSend: (text) => {
      if (entry.runtime !== runtime) return;
      entry.dirty = true;
      entry.onSend?.(text, entry.start.code);
    },
    onStopped: (reason) => {
      if (reason === "closed" || entry.runtime !== runtime) return;
      entry.runtime = null;
      entry.batches = [];
      entry.stopped = reason;
      changed();
    },
  });
  entry.runtime = runtime;
  if (entry.views.length === 0 || document.hidden) runtime.suspend();
}

function relaunch(entry: Entry, start: AppStart): void {
  entry.runtime?.terminate();
  entry.runtime = null;
  entry.kept = null;
  entry.start = { ...start, context: entry.start.context };
  launch(entry);
  changed();
}

function stopEntry(entry: Entry, reason: string): void {
  entry.runtime?.terminate();
  entry.runtime = null;
  entry.batches = [];
  entry.stopped = reason;
  changed();
}

function discard(entry: Entry): void {
  if (entry.release) clearTimeout(entry.release);
  entry.runtime?.terminate();
  entry.runtime = null;
  entries.delete(entry.key);
  changed();
}

const live = (): number => [...entries.values()].filter((entry) => entry.runtime !== null).length;

/** The off-screen app that makes room for a visible one: an idle app
 *  first, else the typed-into app hidden longest. */
const evictable = (): Entry | undefined =>
  [...entries.values()]
    .filter((entry) => entry.runtime !== null && entry.views.length === 0)
    .sort((a, b) => Number(a.dirty) - Number(b.dirty) || a.hiddenAt - b.hiddenAt)[0];

/** Whether one more runtime may start, evicting an off-screen app if the page is full. */
function admit(): boolean {
  if (live() < MAX_LIVE) return true;
  const idle = evictable();
  if (!idle) return false;
  discard(idle);
  return true;
}

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => {
    for (const entry of entries.values()) {
      if (document.hidden) entry.runtime?.suspend();
      else if (entry.views.length > 0) entry.runtime?.resume();
    }
  });
}

// ---- Views -------------------------------------------------------------------

export type AppView = {
  tree: AppTreeState;
  stopped: string | null;
  primary: boolean;
  /** The page already runs the maximum number of apps. */
  waiting: boolean;
  /** This view's code is newer than the running code the person typed into. */
  outdated: boolean;
  dispatch: (id: number, name: string, payload: unknown) => void;
  reload: () => void;
  /** Keep the running version; the prompt returns for a later version. */
  keep: () => void;
};

function newEntry(key: string, scope: string, start: AppStart): Entry {
  return {
    key,
    scope,
    runtime: null,
    tree: emptyAppTree(),
    stopped: null,
    views: [],
    dirty: false,
    kept: null,
    onSend: undefined,
    release: null,
    hiddenAt: 0,
    lastCredit: 0,
    start,
    batches: [],
  };
}

const subscribe = (listener: () => void): (() => void) => {
  pageListeners.add(listener);
  return () => void pageListeners.delete(listener);
};

/** Registers one view of `key` and starts the runtime if it is the first. */
function useViewRegistration(key: string, scope: string, start: AppStart, view: symbol): boolean {
  const [waiting, setWaiting] = useState(false);
  useEffect(() => {
    let entry = entries.get(key);
    if (!entry) {
      if (!admit()) {
        setWaiting(true);
        const retry = (): void => {
          if (live() < MAX_LIVE || evictable()) {
            pageListeners.delete(retry);
            setWaiting(false);
          }
        };
        pageListeners.add(retry);
        return () => void pageListeners.delete(retry);
      }
      entry = newEntry(key, scope, start);
      entries.set(key, entry);
      entry.views.push(view);
      launch(entry);
    } else {
      if (entry.release) clearTimeout(entry.release);
      entry.release = null;
      if (entry.views.length === 0 && !document.hidden) entry.runtime?.resume();
      entry.views.push(view);
    }
    changed();
    const owned = entry;
    return () => {
      owned.views = owned.views.filter((other) => other !== view);
      changed();
      if (owned.views.length > 0) return;
      // Off screen the app pauses; an untouched one ends after a grace, while
      // one the person typed into lives on until a visible app needs its room.
      owned.hiddenAt = performance.now();
      owned.runtime?.suspend();
      owned.release = setTimeout(() => {
        owned.release = null;
        if (!owned.dirty || !owned.runtime) discard(owned);
      }, RELEASE_GRACE_MS);
    };
    // `start` only seeds a new runtime; later versions are handled by the view.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, view, waiting]);
  return waiting;
}

/** A view of the app `key` (session view `scope`), starting it if needed.
 *  `start` is the version this view was given; sends report the running one. */
export function useAppRuntime(
  key: string,
  scope: string,
  start: AppStart,
  onSend: ((text: string, code: string) => void) | undefined,
): AppView {
  const view = useMemo(() => Symbol(key), [key]);
  const waiting = useViewRegistration(key, scope, start, view);
  useSyncExternalStore(subscribe, () => version.value);
  const entry = entries.get(key);
  const primary = entry ? entry.views.at(-1) === view : false;
  if (entry && primary) entry.onSend = onSend;
  const current = entry?.start.code === start.code;
  const offered = entry !== undefined && !current && entry.kept !== start.code;
  const propsJson = JSON.stringify(start.props ?? {});
  const contextJson = JSON.stringify(start.context ?? {});

  // A newer version replaces an untouched or stopped app at once.
  useEffect(() => {
    if (!entry || !primary || !offered || (entry.dirty && entry.runtime)) return;
    if (entry.runtime || admit()) relaunch(entry, start);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entry, primary, offered, start.code]);
  // Props only reach the code they were written for.
  useEffect(() => {
    if (!entry || !primary || !current) return;
    entry.start = { ...entry.start, props: JSON.parse(propsJson) as unknown };
    entry.runtime?.props(entry.start.props);
  }, [entry, primary, current, propsJson]);
  useEffect(() => {
    if (!entry || !primary) return;
    entry.start = { ...entry.start, context: JSON.parse(contextJson) as unknown };
    entry.runtime?.context(entry.start.context);
  }, [entry, primary, contextJson]);

  return {
    tree: entry?.tree ?? emptyAppTree(),
    stopped: entry?.stopped ?? null,
    primary,
    waiting,
    outdated: offered && entry.dirty && entry.runtime !== null,
    dispatch: (id, name, payload) => {
      if (!entry?.runtime) return;
      if (name === "onChange") entry.dirty = true;
      entry.runtime.event(id, name, payload);
    },
    reload: () => {
      if (!entry) return;
      if (!entry.runtime && !admit()) {
        entry.stopped = "Too many apps are running on this page. Close one, then reload.";
        changed();
        return;
      }
      relaunch(entry, start);
    },
    keep: () => {
      if (!entry) return;
      entry.kept = start.code;
      changed();
    },
  };
}

/** Stops the runtime of `key` with a person-readable reason (renderer failure). */
export function stopAppRuntime(key: string, reason: string): void {
  const entry = entries.get(key);
  if (entry) stopEntry(entry, reason);
}
