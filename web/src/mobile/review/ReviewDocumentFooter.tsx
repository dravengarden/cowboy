import type { ComponentProps } from "react";
import { createPortal } from "react-dom";
import { KeyboardArrowDown, KeyboardArrowUp } from "@mui/icons-material";
import { IconButton, Typography } from "@mui/material";
import { ReviewCheckStatus } from "./ReviewCheckStatus";
import { ReviewDocumentControls } from "./ReviewDocumentControls";
import { type ReviewDocumentSlots, useReviewDocumentSlots } from "./reviewDocumentSlots";

/** Checks and document navigation share one row, including short viewports.
 *  A diff inside the Review file controls moves its line counts and hunk
 *  navigation into that row and its check actions into More actions. */
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
  const slots = useReviewDocumentSlots();
  if (!visible && !checks) return null;
  const merged = controls.diff && slots ? slots : undefined;
  return (
    <>
      {merged && <DiffSlotControls slots={merged} {...controls} />}
      <ReviewDocumentControls
        {...controls}
        {...(merged ? { counts: undefined, hunkCount: 0 } : {})}
      >
        {checks && (
          <ReviewCheckStatus
            sessionId={sessionId}
            path={path}
            {...(merged ? { actionsContainer: merged.actions } : {})}
          />
        )}
      </ReviewDocumentControls>
    </>
  );
}

function DiffSlotControls({
  slots,
  counts,
  hunkCount,
  hunkIndex,
  onPrevious,
  onNext,
}: ComponentProps<typeof ReviewDocumentControls> & {
  slots: ReviewDocumentSlots;
}): React.JSX.Element {
  return (
    <>
      {counts && slots.counts && createPortal(
        <>
          <Typography variant="caption" color="success.main" lineHeight="inherit">
            +{counts.added}
          </Typography>
          <Typography variant="caption" color="error.main" lineHeight="inherit">
            −{counts.removed}
          </Typography>
        </>,
        slots.counts,
      )}
      {hunkCount > 1 && slots.hunks && createPortal(
        <>
          {/* A phone row only fits one hunk control; Next wraps around. */}
          <IconButton
            aria-label="Previous hunk"
            disabled={hunkIndex <= 0}
            onClick={onPrevious}
            sx={{ "@media (max-width: 599px)": { display: "none" } }}
          >
            <KeyboardArrowUp />
          </IconButton>
          <IconButton
            aria-label="Next hunk"
            title={`Hunk ${hunkIndex + 1} of ${hunkCount}`}
            data-review-hunk-position={`${hunkIndex + 1}/${hunkCount}`}
            onClick={onNext}
          >
            <KeyboardArrowDown />
          </IconButton>
        </>,
        slots.hunks,
      )}
    </>
  );
}
