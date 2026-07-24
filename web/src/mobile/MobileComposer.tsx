import { memo } from "react";
import { ComposerWorkspace } from "../Composer";
import type { ComposerWorkspaceProps } from "../composer/contracts";

export const MobileComposer = memo(function MobileComposer({
  sessionId,
  status,
  autoFocus = false,
  onSubmitted,
}: Omit<ComposerWorkspaceProps, "variant">): React.JSX.Element {
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
