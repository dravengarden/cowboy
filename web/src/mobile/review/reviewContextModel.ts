import type { SessionMeta } from "../../protocol";
import { sessionProjectLabel } from "../../sessionProject";

export interface ReviewContextWorktree {
  readonly key: string;
  readonly path: string;
  readonly label: string;
  readonly sessions: readonly SessionMeta[];
}

export interface ReviewContextProject {
  readonly key: string;
  readonly label: string;
  readonly machineId: string;
  readonly registeredWorkspace?: ReviewRegisteredWorkspace;
  readonly worktrees: readonly ReviewContextWorktree[];
  readonly sessions: readonly SessionMeta[];
}

export interface ReviewRegisteredWorkspace {
  readonly id: string;
  readonly displayName: string;
  readonly canonicalPath: string;
}

function fallbackProject(session: SessionMeta): string {
  const normalized = session.cwd.replace(/\/+$/, "");
  return normalized.split("/").at(-1) || session.cwd;
}

export function reviewSessionProject(session: SessionMeta): string {
  return sessionProjectLabel(session) ?? fallbackProject(session);
}

export function worktreeLabel(path: string, sourcePath?: string): string {
  const normalized = path.replace(/\/+$/, "");
  const source = sourcePath?.replace(/\/+$/, "");
  if (source && normalized === source) return "Shared workspace";
  return normalized.split("/").at(-1) || path;
}

/**
 * Build one Machine's phone context hierarchy from its registered workspace
 * inventory, then decorate it with the newest-first session snapshot. The
 * registered workspace is a trusted creation source, not a session worktree.
 */
export function buildReviewContextProjects(
  sessions: readonly SessionMeta[],
  machineId?: string,
  registeredWorkspaces: readonly ReviewRegisteredWorkspace[] = [],
): readonly ReviewContextProject[] {
  const selectedMachineId = machineId ?? "local";
  const projects = new Map<
    string,
    {
      label: string;
      machineId: string;
      registeredWorkspace?: ReviewRegisteredWorkspace;
      sessions: SessionMeta[];
      worktrees: Map<string, {
        path: string;
        label: string;
        sessions: SessionMeta[];
      }>;
    }
  >();

  for (const workspace of registeredWorkspaces) {
    if (workspace.id === "home") continue;
    const projectKey = `${selectedMachineId}\u0000${workspace.displayName}`;
    projects.set(projectKey, {
      label: workspace.displayName,
      machineId: selectedMachineId,
      registeredWorkspace: workspace,
      sessions: [],
      worktrees: new Map(),
    });
  }

  for (const session of sessions) {
    const sessionMachineId = session.machine_id ?? "local";
    if (sessionMachineId !== selectedMachineId) continue;
    const label = reviewSessionProject(session);
    const projectKey = `${sessionMachineId}\u0000${label}`;
    let project = projects.get(projectKey);
    if (!project) {
      project = {
        label,
        machineId: sessionMachineId,
        sessions: [],
        worktrees: new Map(),
      };
      projects.set(projectKey, project);
    }
    project.sessions.push(session);

    const worktreeKey = `${sessionMachineId}\u0000${session.cwd}`;
    let worktree = project.worktrees.get(worktreeKey);
    if (!worktree) {
      worktree = {
        path: session.cwd,
        label: worktreeLabel(session.cwd, session.workspace_source_path),
        sessions: [],
      };
      project.worktrees.set(worktreeKey, worktree);
    }
    worktree.sessions.push(session);
  }

  return [...projects.entries()].map(([key, project]) => ({
    key,
    label: project.label,
    machineId: project.machineId,
    ...(project.registeredWorkspace
      ? { registeredWorkspace: project.registeredWorkspace }
      : {}),
    sessions: project.sessions,
    worktrees: [...project.worktrees.entries()].map(([worktreeKey, worktree]) => ({
      key: worktreeKey,
      path: worktree.path,
      label: worktree.label,
      sessions: worktree.sessions,
    })),
  }));
}

export function orderReviewContextProjects(
  projects: readonly ReviewContextProject[],
  currentProject: string | undefined,
  currentMachineId: string | undefined,
): readonly ReviewContextProject[] {
  return [...projects].sort((left, right) => {
    const leftCurrent = left.label === currentProject &&
      left.machineId === (currentMachineId ?? "local");
    const rightCurrent = right.label === currentProject &&
      right.machineId === (currentMachineId ?? "local");
    if (leftCurrent !== rightCurrent) return leftCurrent ? -1 : 1;
    return left.label.localeCompare(right.label) ||
      left.machineId.localeCompare(right.machineId);
  });
}

export function pushReviewSessionHistory(
  history: readonly string[],
  currentId: string | undefined,
  targetId: string,
): readonly string[] {
  if (!currentId || currentId === targetId) return history;
  return [...history.filter((id) => id !== currentId), currentId].slice(-16);
}

export function previousReviewSessionId(
  history: readonly string[],
  currentId: string | undefined,
  validIds: ReadonlySet<string>,
): string | undefined {
  return [...history].reverse().find((id) =>
    id !== currentId && validIds.has(id)
  );
}

export function popReviewSessionHistory(
  history: readonly string[],
  targetId: string,
): readonly string[] {
  const index = history.lastIndexOf(targetId);
  return index < 0 ? history : history.slice(0, index);
}
