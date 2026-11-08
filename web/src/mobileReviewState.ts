import { validReviewDirectory } from "./reviewDirectoryIdentity";
import { type Mutators } from "@cowboy/state-sync";

export interface MobileReviewTabState {
  readonly path: string;
  readonly pinned: boolean;
}

export interface MobileReviewState {
  readonly open_revision?: number;
  readonly reveal_revision?: number;
  /** What the latest reveal named, bound to the revision it set. */
  readonly reveal?: { readonly path: string; readonly line?: number; readonly revision: number };
  readonly directory?: string | null;
  readonly remote_review?: import("./review/remoteReviewModel").RemoteReviewBinding | null;
  readonly remote_selected?: boolean;
  /** `""` is uncommitted work; absent follows the repository default. */
  readonly comparison?: string;
  /** Other code contexts this session works in. */
  readonly bindings?: readonly string[];
  readonly mode: "files" | "git";
  readonly tabs: readonly MobileReviewTabState[];
  readonly active?: string;
  readonly progress: Readonly<Record<string, string>>;
  readonly positions: Readonly<
    Record<string, { readonly line: number; readonly revision?: string }>
  >;
}

export const mobileReviewMutators = {
  setDirectory: (value: MobileReviewState, args: { directory?: string | null; inherit?: boolean }): MobileReviewState => {
    if ((args.inherit && args.directory !== undefined) || (!args.inherit && args.directory !== null && !validReviewDirectory(args.directory))) throw new Error("Invalid Review directory");
    const next = { ...value };
    if (args.inherit) delete next.directory;
    else if (args.directory !== undefined) return { ...value, directory: args.directory };
    return next;
  },
  setRemoteReview: (
    value: MobileReviewState,
    args: { binding: import("./review/remoteReviewModel").RemoteReviewBinding | null },
  ): MobileReviewState => ({ ...value, remote_review: args.binding }),
  selectRemoteReview: (
    value: MobileReviewState, args: { selected: boolean },
  ): MobileReviewState => ({ ...value, remote_selected: args.selected }),
  setComparison: (
    value: MobileReviewState, args: { comparison: string | null },
  ): MobileReviewState => {
    const { comparison: _comparison, ...rest } = value;
    return args.comparison === null ? rest : { ...rest, comparison: args.comparison };
  },
  bind: (value: MobileReviewState, args: { context: string }): MobileReviewState => ({
    ...value,
    bindings: [...new Set([...(value.bindings ?? []), args.context])],
  }),
  unbind: (value: MobileReviewState, args: { context: string }): MobileReviewState => {
    const { bindings = [], ...rest } = value;
    const kept = bindings.filter((context) => context !== args.context);
    return kept.length > 0 ? { ...rest, bindings: kept } : rest;
  },
  open: (value: MobileReviewState, args: { path: string; reveal?: boolean; line?: number }): MobileReviewState => {
    if (args.line !== undefined && (!args.reveal || !Number.isInteger(args.line) || args.line < 1)) {
      throw new Error("Invalid reveal line");
    }
    const existing = value.tabs.find((tab) => tab.path === args.path);
    let tabs = existing ? [...value.tabs] : [...value.tabs, { path: args.path, pinned: false }];
    if (tabs.length > 12) {
      const evict = tabs.findIndex((tab) => !tab.pinned);
      tabs = tabs.filter((_, index) => index !== (evict < 0 ? 0 : evict));
    }
    const revision = (value.reveal_revision ?? 0) + 1;
    return { ...value, mode: "files", tabs, active: args.path, open_revision: (value.open_revision ?? 0) + 1,
      ...(args.reveal
        ? {
          reveal_revision: revision,
          remote_selected: false,
          reveal: { path: args.path, ...(args.line === undefined ? {} : { line: args.line }), revision },
        }
        : {}) };
  },
  close: (value: MobileReviewState, args: { path: string }): MobileReviewState => {
    const tabs = value.tabs.filter((tab) => tab.path !== args.path);
    const active = value.active === args.path ? tabs.at(-1)?.path : value.active;
    const { active: _active, ...rest } = value;
    return {
      ...rest,
      tabs,
      ...(active === undefined ? {} : { active }),
    };
  },
  reorder: (value: MobileReviewState, args: { paths: readonly string[] }): MobileReviewState => {
    const position = new Map(args.paths.map((path, index) => [path, index]));
    return {
      ...value,
      tabs: [...value.tabs].sort((left, right) =>
        (position.get(left.path) ?? Number.MAX_SAFE_INTEGER) -
        (position.get(right.path) ?? Number.MAX_SAFE_INTEGER)
      ),
    };
  },
  setPinned: (
    value: MobileReviewState,
    args: { path: string; pinned: boolean },
  ): MobileReviewState => ({
    ...value,
    tabs: value.tabs.map((tab) =>
      tab.path === args.path ? { ...tab, pinned: args.pinned } : tab
    ),
  }),
  activate: (
    value: MobileReviewState,
    args: { path: string | null },
  ): MobileReviewState => {
    const { active: _active, ...rest } = value;
    return {
      ...rest,
      ...(args.path && value.tabs.some((tab) => tab.path === args.path)
        ? { active: args.path }
        : {}),
    };
  },
  setMode: (
    value: MobileReviewState,
    args: { mode: "files" | "git" },
  ): MobileReviewState => ({ ...value, mode: args.mode }),
  markReviewed: (
    value: MobileReviewState,
    args: { key: string; revision: string | null },
  ): MobileReviewState => {
    const progress = { ...value.progress };
    if (args.revision === null) delete progress[args.key];
    else progress[args.key] = args.revision;
    return { ...value, progress };
  },
  setPosition: (
    value: MobileReviewState,
    args: { path: string; line: number; revision: string | null },
  ): MobileReviewState => ({
    ...value,
    positions: {
      ...value.positions,
      [args.path]: {
        line: Math.max(1, Math.trunc(args.line)),
        ...(args.revision === null ? {} : { revision: args.revision }),
      },
    },
  }),
} satisfies Mutators<MobileReviewState>;

/** The file (and line) the latest reveal asked for. A reveal payload bound to
 *  the current `reveal_revision` wins; a writer that predates it leaves only
 *  `active`, which is then read as the documented narrow fallback. */
export function revealedFile(state: MobileReviewState): { path: string; line?: number } | undefined {
  const reveal = state.reveal;
  if (reveal && reveal.revision === state.reveal_revision) {
    return reveal.line === undefined ? { path: reveal.path } : { path: reveal.path, line: reveal.line };
  }
  return state.active === undefined ? undefined : { path: state.active };
}
