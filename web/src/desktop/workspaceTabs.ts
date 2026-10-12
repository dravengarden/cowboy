// Desktop's tab strip (FOCUS.md "Tabs"): the Sessions and Drafts kept open
// on this device, in strip order. Keys are workspace item keys: a session id
// or `draft:<id>`. Per device and never synced; Mobile has no tabs.
//
// Like VS Code and Zed, an item opened by browsing takes the single preview
// tab and the next one opened replaces it. Typing in it, keeping or pinning
// it makes the tab permanent. Pinned tabs lead the strip and survive every
// bulk close.

import { useSyncExternalStore } from "react";

const STORAGE_KEY = "cowboy.desktop.tabs.v1";
export const DESKTOP_TAB_LIMIT = 40;
const CLOSED_LIMIT = 20;

export interface DesktopTab {
  readonly key: string;
  readonly pinned: boolean;
  /** Replaced by the next item opened unless it is kept first. */
  readonly preview: boolean;
}

export interface DesktopTabs {
  readonly tabs: readonly DesktopTab[];
  /** Closed keys, most recent last, for Reopen. */
  readonly closed: readonly string[];
}

export const NO_DESKTOP_TABS: DesktopTabs = { tabs: [], closed: [] };

function pinnedCount(tabs: readonly DesktopTab[]): number {
  return tabs.filter((tab) => tab.pinned).length;
}

function inserted(
  tabs: readonly DesktopTab[],
  tab: DesktopTab,
  after: string | null,
): DesktopTab[] {
  const anchor = tabs.findIndex((candidate) => candidate.key === after);
  const at = Math.max(
    pinnedCount(tabs),
    anchor === -1 ? tabs.length : anchor + 1,
  );
  const next = [...tabs.slice(0, at), tab, ...tabs.slice(at)];
  // Past the limit the leftmost unpinned tab makes room.
  while (next.length > DESKTOP_TAB_LIMIT) {
    const drop = next.findIndex((candidate) =>
      !candidate.pinned && candidate.key !== tab.key
    );
    if (drop === -1) break;
    next.splice(drop, 1);
  }
  return next;
}

/** Give `key` a tab. An existing tab is left alone; otherwise the preview tab
 * is replaced in place, or a new preview tab opens right of `after`. */
export function withDesktopTab(
  state: DesktopTabs,
  key: string,
  after: string | null = null,
): DesktopTabs {
  if (state.tabs.some((tab) => tab.key === key)) return state;
  const opened: DesktopTab = { key, pinned: false, preview: true };
  const preview = state.tabs.findIndex((tab) => tab.preview);
  if (preview !== -1) {
    return {
      ...state,
      tabs: state.tabs.map((tab, index) => index === preview ? opened : tab),
    };
  }
  return { ...state, tabs: inserted(state.tabs, opened, after) };
}

export function keepDesktopTab(state: DesktopTabs, key: string): DesktopTabs {
  if (!state.tabs.some((tab) => tab.key === key && tab.preview)) return state;
  return {
    ...state,
    tabs: state.tabs.map((tab) =>
      tab.key === key ? { ...tab, preview: false } : tab
    ),
  };
}

/** Pin to the end of the pinned group, or unpin to the start of the rest. */
export function toggleDesktopTabPin(
  state: DesktopTabs,
  key: string,
): DesktopTabs {
  const tab = state.tabs.find((candidate) => candidate.key === key);
  if (!tab) return state;
  const rest = state.tabs.filter((candidate) => candidate.key !== key);
  const at = pinnedCount(rest);
  const next: DesktopTab = { key, pinned: !tab.pinned, preview: false };
  return { ...state, tabs: [...rest.slice(0, at), next, ...rest.slice(at)] };
}

/** Move within the tab's own group: pinned tabs never mix with the rest. */
export function moveDesktopTab(
  state: DesktopTabs,
  key: string,
  to: number,
): DesktopTabs {
  const from = state.tabs.findIndex((tab) => tab.key === key);
  if (from === -1) return state;
  const tab = state.tabs[from]!;
  const pinned = pinnedCount(state.tabs);
  const [low, high] = tab.pinned ? [0, pinned - 1] : [pinned, state.tabs.length - 1];
  const target = Math.min(high, Math.max(low, to));
  if (target === from) return state;
  const tabs = state.tabs.filter((candidate) => candidate.key !== key);
  tabs.splice(target, 0, { ...tab, preview: false });
  return { ...state, tabs };
}

export type DesktopTabCloseScope = "one" | "others" | "left" | "right" | "all";

/** The keys a close command removes. Only closing one tab removes a pinned
 * one; every bulk close leaves pinned tabs. */
export function desktopTabsToClose(
  tabs: readonly DesktopTab[],
  key: string,
  scope: DesktopTabCloseScope,
): string[] {
  const at = tabs.findIndex((tab) => tab.key === key);
  if (scope === "one") return at === -1 ? [] : [key];
  return tabs.filter((tab, index) => {
    if (tab.pinned) return false;
    if (scope === "all") return true;
    if (at === -1) return false;
    if (scope === "others") return index !== at;
    return scope === "left" ? index < at : index > at;
  }).map((tab) => tab.key);
}

export function withoutDesktopTabs(
  state: DesktopTabs,
  keys: readonly string[],
): DesktopTabs {
  const gone = new Set(keys);
  const removed = state.tabs.filter((tab) => gone.has(tab.key));
  if (removed.length === 0) return state;
  return {
    tabs: state.tabs.filter((tab) => !gone.has(tab.key)),
    closed: [
      ...state.closed.filter((key) => !gone.has(key)),
      ...removed.map((tab) => tab.key),
    ].slice(-CLOSED_LIMIT),
  };
}

/** The tab to show after `closing` are gone: the most recently visited one
 * that remains, else the nearest neighbour of the current tab. */
export function desktopTabAfterClose(
  tabs: readonly DesktopTab[],
  current: string,
  closing: readonly string[],
  visits: readonly string[],
): string | null {
  const gone = new Set(closing);
  if (!gone.has(current)) return current;
  const remaining = tabs.filter((tab) => !gone.has(tab.key));
  if (remaining.length === 0) return null;
  const visited = visits.find((key) =>
    remaining.some((tab) => tab.key === key)
  );
  if (visited) return visited;
  const at = tabs.findIndex((tab) => tab.key === current);
  for (let distance = 1; distance < tabs.length; distance++) {
    for (const index of [at + distance, at - distance]) {
      const tab = tabs[index];
      if (tab && !gone.has(tab.key)) return tab.key;
    }
  }
  return remaining[0]!.key;
}

/** Reopen the most recently closed item that still exists, as a kept tab
 * right of `after`. The preview tab is left alone: reopening is deliberate. */
export function reopenDesktopTab(
  state: DesktopTabs,
  exists: (key: string) => boolean,
  after: string | null = null,
): { state: DesktopTabs; key: string | null } {
  const closed = [...state.closed];
  while (closed.length > 0) {
    const key = closed.pop()!;
    if (!exists(key) || state.tabs.some((tab) => tab.key === key)) continue;
    const tab: DesktopTab = { key, pinned: false, preview: false };
    return { state: { closed, tabs: inserted(state.tabs, tab, after) }, key };
  }
  return { state: { ...state, closed }, key: null };
}

/** The tab `delta` steps from `current`, wrapping at the ends. */
export function desktopTabStep(
  tabs: readonly DesktopTab[],
  current: string | null,
  delta: number,
): string | null {
  if (tabs.length === 0) return null;
  const at = tabs.findIndex((tab) => tab.key === current);
  if (at === -1) return tabs[delta > 0 ? 0 : tabs.length - 1]!.key;
  return tabs[(at + delta + tabs.length) % tabs.length]!.key;
}

/** Drop tabs whose item is gone (deleted elsewhere). */
export function pruneDesktopTabs(
  state: DesktopTabs,
  exists: (key: string) => boolean,
): DesktopTabs {
  if (state.tabs.every((tab) => exists(tab.key))) return state;
  return { ...state, tabs: state.tabs.filter((tab) => exists(tab.key)) };
}

function parse(raw: string | null): DesktopTabs {
  try {
    const value = JSON.parse(raw ?? "null") as Partial<DesktopTabs> | null;
    if (!value || !Array.isArray(value.tabs)) return NO_DESKTOP_TABS;
    const seen = new Set<string>();
    const tabs = value.tabs.filter((tab): tab is DesktopTab =>
      typeof tab === "object" && tab !== null &&
      typeof tab.key === "string" && typeof tab.pinned === "boolean" &&
      typeof tab.preview === "boolean" && !seen.has(tab.key) &&
      seen.add(tab.key) !== undefined
    ).slice(0, DESKTOP_TAB_LIMIT);
    // Enforce the invariants a hand-edited store could break.
    let preview = false;
    const ordered = [
      ...tabs.filter((tab) => tab.pinned),
      ...tabs.filter((tab) => !tab.pinned),
    ].map((tab) => {
      const keep = tab.preview && !tab.pinned && !preview;
      preview ||= keep;
      return { key: tab.key, pinned: tab.pinned, preview: keep };
    });
    const closed = Array.isArray(value.closed)
      ? value.closed.filter((key): key is string => typeof key === "string")
        .slice(-CLOSED_LIMIT)
      : [];
    return { tabs: ordered, closed };
  } catch {
    return NO_DESKTOP_TABS;
  }
}

function storage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

let current: DesktopTabs = parse(storage()?.getItem(STORAGE_KEY) ?? null);
const listeners = new Set<() => void>();

export function desktopTabs(): DesktopTabs {
  return current;
}

export function updateDesktopTabs(
  update: (state: DesktopTabs) => DesktopTabs,
): void {
  const next = update(current);
  if (next === current) return;
  current = next;
  try {
    storage()?.setItem(STORAGE_KEY, JSON.stringify(current));
  } catch {
    // A full or blocked store only costs persistence across reloads.
  }
  for (const listener of listeners) listener();
}

export function useDesktopTabs(): DesktopTabs {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => current,
    () => current,
  );
}
