import { ComposerWorkspace } from "../Composer";
import type { ComposerWorkspaceProps } from "../composer/contracts";

export function DesktopComposer({
  sessionId,
  status,
  variant = "overlay",
}: ComposerWorkspaceProps): React.JSX.Element {
  return (
    <ComposerWorkspace
      sessionId={sessionId}
      status={status}
      variant={variant}
      surface="desktop"
    />
  );
}
