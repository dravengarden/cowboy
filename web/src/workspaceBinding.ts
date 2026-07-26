import type { SessionMeta } from "./protocol";

export interface WorkspaceBinding {
  readonly sessionId: string;
  readonly cwd: string;
  readonly provider: string;
  readonly title: string;
}

export function resolveWorkspaceBinding(
  sessions: readonly SessionMeta[],
  selectedSessionId: string | null,
): WorkspaceBinding | null {
  const session = sessions.find(({ id }) => id === selectedSessionId) ??
    sessions[0];
  return session
    ? {
      sessionId: session.id,
      cwd: session.cwd,
      provider: session.provider,
      title: session.title,
    }
    : null;
}

