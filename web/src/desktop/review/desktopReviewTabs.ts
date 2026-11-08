// Desktop Review's open files (docs/desktop-code-review.md "State
// ownership"): per device and per reviewed context, never synced, so the
// phone's Review tabs are its own. Like the Session tabs, a file opened by
// browsing takes the single preview tab; keeping or pinning makes it stay.

import type { CodeDiffScope } from "../../mobile/review/codeApi";

export type DesktopReviewDocument =
  | {
    readonly kind: "diff";
    readonly path: string;
    readonly scope: CodeDiffScope;
    readonly comparison?: string | undefined;
  }
  | { readonly kind: "source"; readonly path: string }
  /** One file of a commit, from History: its diff against the first parent. */
  | { readonly kind: "commit"; readonly oid: string; readonly path: string }
  /** One file of a remote pull request at one revision. Its patch lives only
   *  in memory (pullPatches.ts), so these tabs are not restored on reload. */
  | {
    readonly kind: "pr";
    readonly repositoryId: string;
    readonly number: string;
    readonly revision: string;
    readonly path: string;
  };

export interface DesktopReviewTab {
  readonly document: DesktopReviewDocument;
  readonly preview: boolean;
}

export interface DesktopReviewTabs {
  readonly tabs: readonly DesktopReviewTab[];
  readonly active: string | null;
}

export const NO_REVIEW_TABS: DesktopReviewTabs = { tabs: [], active: null };
export const DESKTOP_REVIEW_TAB_LIMIT = 24;

export function reviewDocumentKey(document: DesktopReviewDocument): string {
  if (document.kind === "diff") return `diff\0${document.comparison ?? document.scope}\0${document.path}`;
  if (document.kind === "commit") return `commit\0${document.oid}\0${document.path}`;
  if (document.kind === "pr") {
    return `pr\0${document.repositoryId}\0${document.number}\0${document.revision}\0${document.path}`;
  }
  return `source\0${document.path}`;
}

/** Open `document`: an open tab is selected; otherwise it replaces the preview
 *  tab, or opens beside the active tab. `keep` opens it as a permanent tab. */
export function openReviewDocument(
  state: DesktopReviewTabs,
  document: DesktopReviewDocument,
  keep = false,
): DesktopReviewTabs {
  const key = reviewDocumentKey(document);
  const existing = state.tabs.findIndex((tab) => reviewDocumentKey(tab.document) === key);
  if (existing !== -1) {
    const tabs = keep && state.tabs[existing]!.preview
      ? state.tabs.map((tab, index) => index === existing ? { ...tab, preview: false } : tab)
      : state.tabs;
    return { tabs, active: key };
  }
  const opened: DesktopReviewTab = { document, preview: !keep };
  const preview = state.tabs.findIndex((tab) => tab.preview);
  let tabs: DesktopReviewTab[];
  if (!keep && preview !== -1) {
    tabs = state.tabs.map((tab, index) => index === preview ? opened : tab);
  } else {
    const anchor = state.tabs.findIndex((tab) => reviewDocumentKey(tab.document) === state.active);
    tabs = [...state.tabs];
    tabs.splice(anchor === -1 ? tabs.length : anchor + 1, 0, opened);
  }
  while (tabs.length > DESKTOP_REVIEW_TAB_LIMIT) {
    const drop = tabs.findIndex((tab) => reviewDocumentKey(tab.document) !== key);
    tabs.splice(drop, 1);
  }
  return { tabs, active: key };
}

export function keepReviewTab(state: DesktopReviewTabs, key: string | null): DesktopReviewTabs {
  if (!state.tabs.some((tab) => tab.preview && reviewDocumentKey(tab.document) === key)) return state;
  return {
    ...state,
    tabs: state.tabs.map((tab) =>
      reviewDocumentKey(tab.document) === key ? { ...tab, preview: false } : tab
    ),
  };
}

/** Close `key`; the neighbour to its right (else left) becomes active. */
export function closeReviewTab(state: DesktopReviewTabs, key: string | null): DesktopReviewTabs {
  const index = state.tabs.findIndex((tab) => reviewDocumentKey(tab.document) === key);
  if (index === -1) return state;
  const tabs = state.tabs.filter((_, at) => at !== index);
  if (state.active !== key) return { tabs, active: state.active };
  const next = tabs[index] ?? tabs[index - 1];
  return { tabs, active: next ? reviewDocumentKey(next.document) : null };
}

/** The tab `offset` places from the active one, wrapping (Vim's `gt`/`gT`). */
export function cycleReviewTab(state: DesktopReviewTabs, offset: number): DesktopReviewTabs {
  if (state.tabs.length === 0) return state;
  const index = Math.max(0, state.tabs.findIndex((tab) => reviewDocumentKey(tab.document) === state.active));
  const length = state.tabs.length;
  const next = state.tabs[(((index + offset) % length) + length) % length]!;
  return { ...state, active: reviewDocumentKey(next.document) };
}

/** Vim's `{n}gt`: the nth tab, one-based; out of range leaves it. */
export function nthReviewTab(state: DesktopReviewTabs, n: number): DesktopReviewTabs {
  const tab = state.tabs[n - 1];
  return tab ? { ...state, active: reviewDocumentKey(tab.document) } : state;
}

export function activeReviewDocument(state: DesktopReviewTabs): DesktopReviewDocument | undefined {
  return state.tabs.find((tab) => reviewDocumentKey(tab.document) === state.active)?.document;
}

function validDocument(value: unknown): value is DesktopReviewDocument {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  if (typeof record.path !== "string" || record.path.length === 0) return false;
  if (record.kind === "source") return true;
  if (record.kind === "commit") return typeof record.oid === "string" && /^[0-9a-f]{7,64}$/.test(record.oid);
  return record.kind === "diff" &&
    (record.scope === "combined" || record.scope === "staged" || record.scope === "unstaged") &&
    (record.comparison === undefined || typeof record.comparison === "string");
}

/** Accept only well-formed stored tabs; anything else starts empty. */
export function parseReviewTabs(raw: unknown): DesktopReviewTabs {
  if (!raw || typeof raw !== "object") return NO_REVIEW_TABS;
  const record = raw as Record<string, unknown>;
  if (!Array.isArray(record.tabs)) return NO_REVIEW_TABS;
  const seen = new Set<string>();
  let preview = false;
  const tabs: DesktopReviewTab[] = [];
  for (const item of record.tabs.slice(0, DESKTOP_REVIEW_TAB_LIMIT)) {
    const document = (item as { document?: unknown } | null)?.document;
    if (!validDocument(document)) continue;
    const key = reviewDocumentKey(document);
    if (seen.has(key)) continue;
    seen.add(key);
    const isPreview: boolean = (item as { preview?: unknown }).preview === true && !preview;
    preview ||= isPreview;
    tabs.push({ document, preview: isPreview });
  }
  const active = typeof record.active === "string" && seen.has(record.active)
    ? record.active
    : tabs[0] ? reviewDocumentKey(tabs[0].document) : null;
  return { tabs, active };
}

const STORAGE_PREFIX = "cowboy.desktop.reviewTabs.v1:";

export function loadReviewTabs(context: string): DesktopReviewTabs {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_PREFIX + context);
    return raw ? parseReviewTabs(JSON.parse(raw)) : NO_REVIEW_TABS;
  } catch {
    return NO_REVIEW_TABS;
  }
}

export function saveReviewTabs(context: string, state: DesktopReviewTabs): void {
  try {
    if (state.tabs.length === 0) globalThis.localStorage?.removeItem(STORAGE_PREFIX + context);
    else globalThis.localStorage?.setItem(STORAGE_PREFIX + context, JSON.stringify(state));
  } catch {
    // Storage denial only loses the remembered tabs.
  }
}
