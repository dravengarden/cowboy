import { ComposerWorkspace } from "../Composer";
import type { ComposerWorkspaceProps } from "../composer/contracts";
import { ManagedChildNotice } from "../ManagedCallsDock";
import { managedChildParent } from "../managedCalls";
import { useStoreSelector } from "../store";

export function DesktopComposer({
  sessionId,
  status,
  variant = "overlay",
}: ComposerWorkspaceProps): React.JSX.Element {
  // A managed child is controlled by its parent's call, never this Prompt.
  const parent = useStoreSelector((snapshot) =>
    managedChildParent(
      snapshot.sessions.find((session) => session.id === sessionId),
    )
  );
  if (parent !== null) return <ManagedChildNotice parent={parent} child={sessionId} />;
  return (
    <ComposerWorkspace
      sessionId={sessionId}
      status={status}
      variant={variant}
      surface="desktop"
    />
  );
}
