import { memo } from "react";
import { ComposerWorkspace } from "../Composer";
import type { ComposerWorkspaceProps } from "../composer/contracts";

export const MobileComposer = memo(function MobileComposer({
  sessionId,
  status,
}: Omit<ComposerWorkspaceProps, "variant">): React.JSX.Element {
  return (
    <ComposerWorkspace
      sessionId={sessionId}
      status={status}
      variant="overlay"
      surface="mobile"
    />
  );
});
