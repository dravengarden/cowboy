import { resolveFolderDefaults } from "./folderDefaults";
import {
  effectiveSessionFolder,
  sessionFolderById,
  type SessionFoldersValue,
} from "./sessionFolders";
import type { MachineSummary, SessionMeta } from "./protocol";
import { sessionExecution } from "./sessionExecution";
import {
  registeredReviewWorkspace,
  sessionReviewLocation,
  workspaceCodeContextId,
} from "./mobile/review/reviewContextModel";

export function inheritedReviewDirectory(
  session: SessionMeta,
  tree: SessionFoldersValue,
  own: string | null | undefined,
) {
  const folder = effectiveSessionFolder(session, tree);
  const defaults = resolveFolderDefaults(tree, folder);
  const sourceId = defaults.sources.review_directory;
  const inherited = defaults.values.review_directory;
  return {
    value: own === undefined ? inherited : own,
    inherited,
    folder,
    source: own !== undefined
      ? "This session"
      : sourceId
      ? `Inherited from ${
        sessionFolderById(tree, sourceId)?.name ?? "directory"
      }`
      : "Automatic",
    sourceId: own === undefined ? sourceId : undefined,
  };
}

export interface ReviewDirectoryContext {
  sessionId: string;
  cwd: string;
  provider: string;
  title: string;
  machineId: string;
  workspaceId?: string;
  project: string;
  kind: "Worktree" | "Project code" | "Workspace";
}

/** Resolve against current metadata. A missing explicit target must never
 * quietly open another checkout or the Agent's remote runtime directory. */
export function resolveReviewDirectory(
  value: string | null | undefined,
  owner: SessionMeta,
  sessions: readonly SessionMeta[],
  machines: readonly MachineSummary[],
): { context?: ReviewDirectoryContext; error?: string } {
  const session = value?.startsWith("session::")
    ? sessions.find((s) => s.id === value.slice(9))
    : owner;
  if (!session) {
    return { error: "The selected session is no longer available." };
  }
  const execution = sessionExecution(session);
  const machine = machines.find((m) =>
    execution.machineId ? m.id === execution.machineId : m.local
  );
  const registered = registeredReviewWorkspace(
    session,
    (machine?.workspaces ?? []).map((w) => ({
      id: w.id,
      displayName: w.display_name,
      canonicalPath: w.canonical_path,
    })),
  );
  if (value?.startsWith("workspace::") || value === "project") {
    const [, machineId, workspaceId] = value === "project"
      ? ["workspace", machine?.id, registered?.id]
      : value.split("::");
    const targetMachine = machines.find((m) => m.id === machineId);
    const workspace = targetMachine?.workspaces.find((w) =>
      w.id === workspaceId
    );
    if (!targetMachine || !workspace) {
      return {
        error:
          "The default project directory is unavailable. Choose a directory or restore Automatic.",
      };
    }
    return {
      context: {
        sessionId: workspaceCodeContextId(targetMachine.id, workspace.id),
        cwd: workspace.canonical_path,
        provider: "read-only",
        title: workspace.display_name,
        machineId: targetMachine.id,
        workspaceId: workspace.id,
        project: workspace.display_name,
        kind: "Project code",
      },
    };
  }
  if (execution.state !== "ready" && execution.state !== "local") {
    return { error: "The session workspace is not ready yet." };
  }
  const location = sessionReviewLocation(session, registered);
  if (!execution.cwd && registered) {
    return resolveReviewDirectory("project", session, sessions, machines);
  }
  if (!execution.cwd) {
    return { error: "This session does not have a code directory yet." };
  }
  return {
    context: {
      sessionId: session.id,
      cwd: execution.cwd,
      provider: session.provider,
      title: session.title,
      machineId: machine?.id ?? execution.machineId ?? "local",
      project: location.project,
      kind: location.kind,
      ...(registered ? { workspaceId: registered.id } : {}),
    },
  };
}

export function reviewDirectoryLabel(
  value: string | null | undefined,
  machines: readonly MachineSummary[],
  sessions: readonly SessionMeta[] = [],
): string {
  if (value == null) return "Automatic · session worktree, then project";
  if (value === "project") return "Associated project directory";
  if (value.startsWith("session::")) {
    const session = sessions.find((s) => s.id === value.slice(9));
    if (!session) return "Session worktree unavailable";
    const execution = sessionExecution(session);
    const machine = machines.find((m) =>
      execution.machineId ? m.id === execution.machineId : m.local
    );
    return `${session.title} · ${
      machine?.display_name ?? execution.machineId ?? "Local"
    }`;
  }
  const [, machineId, workspaceId] = value.split("::");
  const machine = machines.find((m) => m.id === machineId);
  const workspace = machine?.workspaces.find((w) => w.id === workspaceId);
  return workspace
    ? `${workspace.display_name} · ${machine!.display_name}`
    : "Project directory unavailable";
}
