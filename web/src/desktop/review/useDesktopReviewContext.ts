import { useMemo } from "react";
import { useActiveWorkspaceBinding } from "../../controlPlane";
import {
  inheritedReviewDirectory,
  resolveReviewDirectory,
  reviewStateKey,
} from "../../reviewDirectoryDefault";
import { sessionExecution } from "../../sessionExecution";
import { useMobileReviewState, useStoreSelector } from "../../store";
import type { MobileReviewState } from "../../mobileReviewState";
import { useReviewContextChoice } from "./desktopReviewContextChoice";

/** The code a Desktop Review reads: the Session's worktree, or the review
 *  directory its folder defaults name (13-code-review "Code surface"). */
export interface DesktopReviewTarget {
  /** Session whose Code API serves the reads (`session::`/`workspace::`/`worktree::` ids included). */
  readonly sessionId: string;
  readonly cwd: string;
  /** Short name of the context for the header. */
  readonly label: string;
  /** Whether this is the Agent's own worktree (remarks then need no root). */
  readonly own: boolean;
  /** Where the files are: the Machine and, for project code, its workspace. */
  readonly machineId: string | undefined;
  readonly workspaceId: string | undefined;
  /** What kind of code this is, for the context control. */
  readonly kind: string;
}

export interface DesktopReviewContext {
  /** The Agent Session the Prompt writes into. */
  readonly agentSessionId: string | undefined;
  readonly target: DesktopReviewTarget | null;
  /** Why there is no target, when one is expected. */
  readonly error: string | undefined;
  /** Synced Review state of the target, for comparison and progress. */
  readonly stateKey: string | undefined;
  readonly synced: MobileReviewState;
  /** The Agent Session's own synced state: its reveal requests. */
  readonly agentSynced: MobileReviewState;
  /** The reader's pick on this device, absent when following the default. */
  readonly choice: string | undefined;
}

export function useDesktopReviewContext(): DesktopReviewContext {
  const binding = useActiveWorkspaceBinding();
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const machines = useStoreSelector((snapshot) => snapshot.machines);
  const folders = useStoreSelector((snapshot) => snapshot.sessionFolders);
  const session = sessions.find((item) => item.id === binding?.sessionId);
  const own = useMobileReviewState(session?.id);
  const choice = useReviewContextChoice(session?.id);
  const ready = useStoreSelector((snapshot) =>
    snapshot.sessionFoldersReady &&
    (!session || snapshot.mobileReviewReady.has(session.id))
  );
  const resolved = useMemo((): { target: DesktopReviewTarget | null; error?: string } => {
    if (!session || !binding) return { target: null };
    if (!ready) return { target: null, error: "Loading review defaults…" };
    const directory = choice ?? inheritedReviewDirectory(session, folders, own.directory).value;
    const result = resolveReviewDirectory(directory, session, sessions, machines);
    if (result.error) return { target: null, error: result.error };
    if (result.context && result.context.sessionId !== session.id) {
      return {
        target: {
          sessionId: result.context.sessionId,
          cwd: result.context.cwd,
          label: result.context.title,
          own: false,
          machineId: result.context.machineId,
          workspaceId: result.context.workspaceId,
          kind: result.context.kind,
        },
      };
    }
    const execution = sessionExecution(session);
    if (execution.state !== "ready" && execution.state !== "local") {
      return {
        target: null,
        error: execution.state === "preparing"
          ? "The session's worktree is being prepared…"
          : "The session's worktree is unavailable.",
      };
    }
    return {
      target: {
        sessionId: session.id,
        cwd: execution.cwd,
        label: execution.cwd.split("/").filter(Boolean).slice(-2).join("/") || session.title,
        own: true,
        machineId: result.context?.machineId ?? execution.machineId,
        workspaceId: result.context?.workspaceId,
        kind: result.context?.kind ?? "Worktree",
      },
    };
  }, [binding, choice, folders, machines, own.directory, ready, session, sessions]);
  const stateKey = resolved.target
    ? reviewStateKey(session?.id, resolved.target.sessionId)
    : undefined;
  const synced = useMobileReviewState(stateKey);
  return {
    agentSessionId: session?.id,
    target: resolved.target,
    error: resolved.error,
    stateKey,
    synced,
    agentSynced: own,
    choice,
  };
}
