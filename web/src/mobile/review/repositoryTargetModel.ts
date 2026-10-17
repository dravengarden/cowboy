import type { PullSummary, RepositoryRemote } from "../../extensions/api";
import type { SessionMeta } from "../../protocol";
import { sessionExecution } from "../../sessionExecution";
import type {
  GitChangeTarget,
  GitTrackedReference,
  GitWorktreeSummary,
} from "./codeApi";

/** One worktree of the repository and the sessions that work in it. */
export interface RepositoryWorktreeRow {
  readonly worktree: GitWorktreeSummary;
  readonly name: string;
  /** Shortened location: at most the last two path segments. */
  readonly location: string;
  readonly sessions: readonly SessionMeta[];
}

export interface RepositoryWorktreeGroups {
  /** The worktree this Review reads. */
  readonly current?: RepositoryWorktreeRow;
  /** Worktrees another visible session works in, live sessions first. */
  readonly sessions: readonly RepositoryWorktreeRow[];
  /** Worktrees no visible session owns. */
  readonly other: readonly RepositoryWorktreeRow[];
}

function normalized(path: string): string {
  return path.length > 1 ? path.replace(/\/+$/, "") : path;
}

export function worktreeLocation(path: string): string {
  const segments = normalized(path).split("/").filter(Boolean);
  if (segments.length <= 2) return normalized(path);
  return `…/${segments.slice(-2).join("/")}`;
}

function live(session: SessionMeta): boolean {
  return session.status === "busy" || session.status === "running" ||
    session.status === "starting";
}

/**
 * File a repository's worktrees by who uses them. Ownership is the session's
 * working directory on the same Machine, nothing else: no directory layout,
 * branch prefix or project registry is consulted.
 */
export function groupRepositoryWorktrees(
  worktrees: readonly GitWorktreeSummary[],
  sessions: readonly SessionMeta[],
  machineId: string | undefined,
): RepositoryWorktreeGroups {
  const machine = machineId ?? "local";
  const byPath = new Map<string, SessionMeta[]>();
  for (const session of sessions) {
    const execution = sessionExecution(session);
    if (execution.state !== "ready" && execution.state !== "local") continue;
    if ((execution.machineId ?? "local") !== machine) continue;
    const key = normalized(execution.cwd);
    const owners = byPath.get(key);
    if (owners) owners.push(session);
    else byPath.set(key, [session]);
  }
  let current: RepositoryWorktreeRow | undefined;
  const owned: RepositoryWorktreeRow[] = [];
  const other: RepositoryWorktreeRow[] = [];
  for (const worktree of worktrees) {
    const path = normalized(worktree.path);
    const row: RepositoryWorktreeRow = {
      worktree,
      name: path.split("/").at(-1) || worktree.path,
      location: worktreeLocation(worktree.path),
      sessions: byPath.get(path) ?? [],
    };
    if (worktree.current && !current) current = row;
    else if (row.sessions.length > 0) owned.push(row);
    else other.push(row);
  }
  // A stable sort: live work first, Git's own order within each half.
  owned.sort((left, right) =>
    Number(right.sessions.some(live)) - Number(left.sessions.some(live))
  );
  return { ...(current ? { current } : {}), sessions: owned, other };
}

export function worktreeTitle(worktree: GitWorktreeSummary): string {
  return worktree.branch ??
    (worktree.detached ? "Detached HEAD" : worktree.bare ? "Bare" : "Worktree");
}

/** `↑2 ↓1`, one side, or an empty string when the two are level. */
export function divergence(reference: GitTrackedReference): string {
  return [
    reference.ahead > 0 ? `↑${reference.ahead}` : "",
    reference.behind > 0 ? `↓${reference.behind}` : "",
  ].filter(Boolean).join(" ");
}

export interface ChangeTargetFact {
  readonly key: "changes" | "upstream" | "base";
  readonly label: string;
  readonly tone: "default" | "attention";
  /** Comparison reference this fact can open in Changes. */
  readonly comparison?: string;
}

/**
 * The few facts that say where a change stands: what is uncommitted, what is
 * not published yet, and how far it is from the branch it merges into.
 */
export function changeTargetFacts(
  target: GitChangeTarget,
): readonly ChangeTargetFact[] {
  const facts: ChangeTargetFact[] = [];
  if (target.changeCount > 0) {
    facts.push({
      key: "changes",
      label: `${target.changeCount} uncommitted`,
      tone: "attention",
    });
  }
  const { upstream, base } = target;
  if (upstream) {
    const delta = divergence(upstream);
    // Level with a published branch is the quiet state; say nothing.
    if (delta && upstream.reference !== base?.reference) {
      facts.push({
        key: "upstream",
        label: `${delta} ${upstream.label}`,
        tone: upstream.ahead > 0 ? "attention" : "default",
      });
    }
  } else if (target.branch && base && base.ahead > 0) {
    facts.push({ key: "upstream", label: "Not pushed", tone: "attention" });
  }
  if (base) {
    const delta = divergence(base);
    facts.push({
      key: "base",
      label: delta ? `${delta} ${base.label}` : `Level with ${base.label}`,
      tone: "default",
      ...(base.ahead > 0 ? { comparison: base.reference } : {}),
    });
  }
  return facts;
}

/** `owner/repository` of every remote, the only places a local branch lives. */
export function remoteRepositories(
  remotes: readonly RepositoryRemote[],
): ReadonlySet<string> {
  return new Set(
    remotes.map((remote) =>
      `${remote.owner}/${remote.repository}`.toLowerCase()
    ),
  );
}

/**
 * Open pull requests keyed by head branch. A pull request counts for a local
 * branch only when its head lives in one of this repository's remotes; a
 * stranger's fork that happens to reuse the branch name does not.
 */
export function pullsByBranch(
  pulls: readonly PullSummary[],
  repositories: ReadonlySet<string>,
): ReadonlyMap<string, PullSummary> {
  const byBranch = new Map<string, PullSummary>();
  for (const pull of pulls) {
    if (
      !pull.headBranch || !pull.headRepository ||
      !repositories.has(pull.headRepository.toLowerCase())
    ) continue;
    // The listing is newest first; keep the most recently updated one.
    if (!byBranch.has(pull.headBranch)) byBranch.set(pull.headBranch, pull);
  }
  return byBranch;
}

/** Pull requests no local worktree has checked out. */
export function pullsWithoutWorktree(
  pulls: readonly PullSummary[],
  byBranch: ReadonlyMap<string, PullSummary>,
  worktrees: readonly GitWorktreeSummary[],
): readonly PullSummary[] {
  const local = new Set(
    worktrees.flatMap((worktree) => {
      const pull = worktree.branch ? byBranch.get(worktree.branch) : undefined;
      return pull ? [pull.url] : [];
    }),
  );
  return pulls.filter((pull) => !local.has(pull.url));
}

export function pullLabel(pull: PullSummary): string {
  return `${pull.draft ? "Draft " : ""}#${pull.number}`;
}
