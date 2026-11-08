import { Alert } from "@mui/material";
import { checkStatusSummary } from "./checkAnnotations";
import { reviewCodeStatusRow, ReviewStatusMessage } from "./ReviewCodeStatus";
import { useReviewChecks } from "./reviewChecksStore";
import { ReviewCheckActions } from "./ReviewCheckActions";

/** What the latest project check runs found in this file; nothing until a
 *  check has run in the session. */
export function ReviewCheckStatus({
  sessionId,
  path,
  actionsContainer,
}: {
  sessionId: string;
  path: string;
  /** Where check actions render instead of this row; null while closed. */
  actionsContainer?: HTMLElement | null;
}): React.JSX.Element | null {
  const status = checkStatusSummary(useReviewChecks(sessionId), path, Date.now());
  return (
    <>
      <ReviewCheckActions
        key={sessionId}
        sessionId={sessionId}
        {...(actionsContainer === undefined ? {} : { container: actionsContainer })}
      />
      {status && (
        <Alert
          data-review-check-status={status.severity}
          severity={status.severity}
          sx={reviewCodeStatusRow}
        >
          <ReviewStatusMessage summary={status.summary} detail={status.detail} />
        </Alert>
      )}
    </>
  );
}
