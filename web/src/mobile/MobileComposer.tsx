import { ComposerWorkspace } from "../Composer";
import type { ComposerWorkspaceProps } from "../composer/contracts";

export function MobileComposer({
  sessionId,
  status,
}: Omit<ComposerWorkspaceProps, "variant">): React.JSX.Element {
  return (
    <ComposerWorkspace
      sessionId={sessionId}
      status={status}
      variant="overlay"
    />
  );
}
