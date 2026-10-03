import type { SessionMeta } from "./protocol";

/** Directory the user selected when creating the session; legacy sessions fall back to cwd. */
export function sessionDisplayDirectory(session: SessionMeta): string {
  return session.workspace_source_path?.trim() || session.cwd;
}

/** Compact list context; old caller-owned directories have no workspace metadata. */
export function sessionListProjectLabel(session: SessionMeta): string {
  const project = sessionProjectLabel(session);
  if (project) return project;

  const directory = sessionDisplayDirectory(session).trim().replace(/\/+$/, "");
  // A generated session directory identifies the task, not its source project.
  if (/\/cowboy-machine\/worktrees(?:\/|$)/.test(directory)) {
    return "Project not recorded";
  }
  return directory.split("/").at(-1) || "Project not recorded";
}

/** Stable project checkout for repository chrome; never substitute a session worktree. */
export function sessionProjectDirectory(
  session: SessionMeta | undefined,
  registeredPath?: string,
): string | null {
  return registeredPath?.trim() || session?.workspace_source_path?.trim() ||
    null;
}

/** Human-readable stable project/workspace identity for session surfaces. */
export function sessionProjectLabel(session: SessionMeta): string | null {
  const workspaceName = session.workspace_name?.trim();
  if (workspaceName) return workspaceName;

  const workspaceId = session.workspace_id?.trim();
  if (workspaceId) return workspaceId;

  for (const path of [session.workspace_source_path, session.cwd]) {
    const match = path?.match(/\/columbus\/projects\/([^/]+)(?:\/|$)/);
    if (match?.[1]) return match[1];
  }
  return null;
}
