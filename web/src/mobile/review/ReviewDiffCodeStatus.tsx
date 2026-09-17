import { Alert, Box, Button, Typography } from "@mui/material";
import {
  ReviewCodeStatus,
  reviewCodeStatusAction,
  reviewCodeStatusRow,
} from "./ReviewCodeStatus.tsx";
import type { OwnedReviewIntelligence } from "./useOwnedReviewBuffer.ts";
import type { useOwnedReviewDiff } from "./useOwnedReviewDiff.ts";

/** Read-only diff chrome: one attention row, no native reload/Apply control. */
export function ReviewDiffCodeStatus({ diff, intelligence }: {
  diff: ReturnType<typeof useOwnedReviewDiff>;
  intelligence: OwnedReviewIntelligence;
}) {
  if (diff.status === "matched") {
    return (
      <ReviewCodeStatus
        intelligence={intelligence}
        diff={{ checkFile: diff.check }}
      />
    );
  }
  if (diff.status !== "mismatch" && diff.status !== "unavailable") {
    return (
      <Box
        component="span"
        data-review-diff-status={diff.status}
        sx={{ display: "none" }}
      />
    );
  }
  const mismatch = diff.status === "mismatch";
  return (
    <Alert
      data-review-diff-status={diff.status}
      severity={mismatch ? "warning" : "info"}
      sx={reviewCodeStatusRow}
      action={
        <Button
          size="small"
          color="inherit"
          onClick={diff.check}
          sx={reviewCodeStatusAction}
        >
          Check file
        </Button>
      }
    >
      <Typography
        variant="body2"
        noWrap
        title={mismatch
          ? "Diff cannot be matched to the complete current file. No positions are queried."
          : "Complete current-file content is unavailable or exceeds the bounded preview. Nothing was reopened or retried automatically."}
      >
        {mismatch
          ? "Diff differs from current file · intelligence hidden"
          : "Diff intelligence unavailable"}
      </Typography>
    </Alert>
  );
}
