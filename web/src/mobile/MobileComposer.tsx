import { memo } from "react";
import { ComposerWorkspace } from "../Composer";
import type { ComposerWorkspaceProps } from "../composer/contracts";
import { ManagedChildNotice } from "../ManagedCallsDock";
import { managedChildParent } from "../managedCalls";
import { useStoreSelector } from "../store";

export const MobileComposer = memo(function MobileComposer({
  sessionId,
  status,
  autoFocus = false,
  onSubmitted,
}: Omit<ComposerWorkspaceProps, "variant">): React.JSX.Element {
  // A managed child is controlled by its parent's call, never this Prompt.
  const parent = useStoreSelector((snapshot) =>
    managedChildParent(
      snapshot.sessions.find((session) => session.id === sessionId),
    )
  );
  if (parent !== null) return <ManagedChildNotice parent={parent} />;
  return (
    <ComposerWorkspace
      sessionId={sessionId}
      status={status}
      autoFocus={autoFocus}
      onSubmitted={onSubmitted}
      variant="overlay"
      surface="mobile"
    />
  );
});
