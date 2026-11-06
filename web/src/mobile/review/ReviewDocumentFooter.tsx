import type { ComponentProps } from "react";
import { ReviewCheckStatus } from "./ReviewCheckStatus";
import { ReviewDocumentControls } from "./ReviewDocumentControls";

/** Checks and document navigation share one row, including short viewports. */
export function ReviewDocumentFooter({
  sessionId,
  path,
  checks,
  visible,
  ...controls
}: ComponentProps<typeof ReviewDocumentControls> & {
  sessionId: string;
  path: string;
  checks: boolean;
  visible: boolean;
}): React.JSX.Element | null {
  if (!visible && !checks) return null;
  return (
    <ReviewDocumentControls {...controls}>
      {checks && <ReviewCheckStatus sessionId={sessionId} path={path} />}
    </ReviewDocumentControls>
  );
}
