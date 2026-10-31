import type { SessionContext } from "../../sessionContext";
import { sessionLinks } from "../../sessionLinks";
import type { SessionMeta } from "../../protocol";
import { sessionProjectLabel } from "../../sessionProject";
import { sessionExecution } from "../../sessionExecution";

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

export function workspaceCodeContextId(
  machineId: string,
  workspaceId: string,
): string {
  return `workspace::${machineId}::${workspaceId}`;
}

const normalizePath = (path: string): string => path.replace(/\/+$/, "") || "/";

/** Only call with the session's workspace Machine inventory. Names are labels,
 * never identities: two registered checkouts may have the same display name. */
export function registeredReviewWorkspace(
  session: SessionMeta,
  workspaces: readonly ReviewRegisteredWorkspace[],
): ReviewRegisteredWorkspace | undefined {
  const execution = sessionExecution(session);
  if (execution.state !== "ready" && execution.state !== "local") {
    return undefined;
  }
  const byId = session.workspace_id
    ? workspaces.find((workspace) => workspace.id === session.workspace_id)
    : undefined;
  if (byId) return byId;
  const path = session.workspace_source_path || execution.cwd;
  return path
    ? workspaces.find((workspace) =>
      normalizePath(workspace.canonicalPath) === normalizePath(path)
    )
    : undefined;
}

export function sessionReviewLocation(
  session: SessionMeta,
  registered?: ReviewRegisteredWorkspace,
): {
  project: string;
  cwd: string;
  kind: "Worktree" | "Project code" | "Workspace";
} {
  const source = session.workspace_source_path || registered?.canonicalPath;
  const execution = sessionExecution(session);
  const usable = execution.state === "ready" || execution.state === "local";
  const cwd = usable ? execution.cwd || source || "" : "";
  return {
    project: registered?.displayName ?? reviewSessionProject(session),
    // The actual session directory always wins, including a caller-owned
    // checkout unknown to the registry. Folder defaults never move live code.
    cwd,
    kind: usable && source
      ? normalizePath(cwd) === normalizePath(source)
        ? "Project code"
        : "Worktree"
      : "Workspace",
  };
}

function projectKey(machineId: string, identity: string): string {
  return `${machineId}\u0000${identity}`;
}

function fallbackProject(session: SessionMeta): string {
  const execution = sessionExecution(session);
  if (execution.state !== "ready" && execution.state !== "local") {
    return "Workspace";
  }
  const normalized = execution.cwd.replace(/\/+$/, "");
  return normalized.split("/").at(-1) || "Workspace";
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
  localMachineId = "local",
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
    projects.set(projectKey(selectedMachineId, workspace.id), {
      label: workspace.displayName,
      machineId: selectedMachineId,
      registeredWorkspace: workspace,
      sessions: [],
      worktrees: new Map(),
    });
  }

  for (const session of sessions) {
    const execution = sessionExecution(session);
    if (execution.state !== "ready" && execution.state !== "local") continue;
    const sessionMachineId = execution.machineId ?? localMachineId;
    if (sessionMachineId !== selectedMachineId) continue;
    const registered = registeredReviewWorkspace(session, registeredWorkspaces);
    const label = registered?.displayName ?? reviewSessionProject(session);
    const key = projectKey(
      sessionMachineId,
      registered?.id ??
        session.workspace_id ??
        normalizePath(session.workspace_source_path || execution.cwd),
    );
    let project = projects.get(key);
    if (!project) {
      project = {
        label,
        machineId: sessionMachineId,
        sessions: [],
        worktrees: new Map(),
      };
      projects.set(key, project);
    }
    project.sessions.push(session);

    if (!execution.cwd) continue;

    const worktreeKey = `${sessionMachineId}\u0000${
      normalizePath(execution.cwd)
    }`;
    let worktree = project.worktrees.get(worktreeKey);
    if (!worktree) {
      worktree = {
        path: execution.cwd,
        label: worktreeLabel(execution.cwd, session.workspace_source_path),
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
    worktrees: [...project.worktrees.entries()].map((
      [worktreeKey, worktree],
    ) => ({
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

/**
 * Sessions this one came from or has read, whose worktrees hold other work.
 * A copy starts from a clean checkout, so this is where its changes usually are.
 */
export function reviewLinkedSessions(
  session: SessionMeta | undefined,
  context: SessionContext | undefined,
  sessions: readonly SessionMeta[],
): SessionMeta[] {
  if (!session) return [];
  const links = [
    ...(context?.links ?? []),
    ...(context?.source_sessions ?? []).filter((source) =>
      !context?.links?.some((link) => link.id === source.id)
    ),
  ];
  const cwd = sessionExecution(session).cwd;
  return sessionLinks(session, links, sessions, [])
    .filter((link) => link.state === "open" && link.id !== session.id)
    .flatMap((link) => {
      const linked = sessions.find((entry) => entry.id === link.id);
      const worktree = linked && sessionExecution(linked).cwd;
      return linked && worktree && worktree !== cwd ? [linked] : [];
    });
}
