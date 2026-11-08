import type { MachineSummary, SessionMeta } from "../../protocol";
import { resolveReviewDirectory } from "../../reviewDirectoryDefault";
import { sessionExecution } from "../../sessionExecution";
import {
  registeredReviewWorkspace,
  reviewSessionProject,
} from "../../review/reviewContextModel";

export interface ReviewContextOption {
  /** The review-directory value; `null` follows the Session's default. */
  readonly value: string | null;
  readonly title: string;
  readonly detail: string;
  readonly group: "session" | "project" | "linked" | "bound" | "project-sessions";
}

const SAME_PROJECT_LIMIT = 12;

function tail(path: string): string {
  return path.split("/").filter(Boolean).slice(-2).join("/") || path;
}

/**
 * What the context control offers for `session`, in order: its own default,
 * its project checkout, the sessions it links, the contexts bound to it, and
 * other sessions of the same project on the same Machine. Each value is
 * resolved now; one that does not resolve is not offered.
 */
export function reviewContextOptions(
  session: SessionMeta,
  sessions: readonly SessionMeta[],
  machines: readonly MachineSummary[],
  linked: readonly SessionMeta[],
  bindings: readonly string[],
): ReviewContextOption[] {
  const options: ReviewContextOption[] = [{
    value: null,
    title: "This session",
    detail: "Its worktree, or its folder's review directory",
    group: "session",
  }];
  const offered = new Set<string>([`session::${session.id}`]);
  const add = (value: string, group: ReviewContextOption["group"], fallback?: string): void => {
    if (offered.has(value)) return;
    const resolved = resolveReviewDirectory(value, session, sessions, machines).context;
    if (!resolved) return;
    offered.add(value);
    options.push({
      value,
      title: fallback ?? resolved.title,
      detail: `${resolved.kind} · ${tail(resolved.cwd)}`,
      group,
    });
  };
  const execution = sessionExecution(session);
  const machine = machines.find((item) => execution.machineId ? item.id === execution.machineId : item.local);
  const registered = registeredReviewWorkspace(session, (machine?.workspaces ?? []).map((workspace) => ({
    id: workspace.id,
    displayName: workspace.display_name,
    canonicalPath: workspace.canonical_path,
  })));
  if (machine && registered) add(`workspace::${machine.id}::${registered.id}`, "project", `${registered.displayName} checkout`);
  for (const other of linked) add(`session::${other.id}`, "linked", other.title);
  for (const value of bindings) add(value, "bound");
  const project = reviewSessionProject(session);
  let same = 0;
  for (const other of sessions) {
    if (same >= SAME_PROJECT_LIMIT) break;
    if (other.id === session.id || other.trashed_at_ms !== undefined) continue;
    const otherExecution = sessionExecution(other);
    if (otherExecution.machineId !== execution.machineId) continue;
    if (reviewSessionProject(other) !== project || !otherExecution.cwd || otherExecution.cwd === execution.cwd) continue;
    const before = options.length;
    add(`session::${other.id}`, "project-sessions", other.title);
    if (options.length > before) same++;
  }
  return options;
}

export const REVIEW_CONTEXT_GROUPS: Readonly<Record<ReviewContextOption["group"], string>> = {
  session: "Default",
  project: "Project",
  linked: "Linked sessions",
  bound: "Bound to this session",
  "project-sessions": "Other sessions of this project",
};
