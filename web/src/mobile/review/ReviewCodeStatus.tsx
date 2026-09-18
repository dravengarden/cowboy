import { Alert, Box, Button, Typography } from "@mui/material";
import { openAppSettings } from "../../appSettings";
import type { OwnedReviewIntelligence } from "./useOwnedReviewBuffer";

export const reviewCodeStatusAction = {
  textTransform: "none",
  fontWeight: 600,
  minWidth: 0,
  px: 1,
  whiteSpace: "nowrap",
} as const;

export const reviewCodeStatusRow = {
  flexShrink: 0,
  borderRadius: 0,
  minHeight: 40,
  px: 1.5,
  py: 0,
  alignItems: "center",
  "& .MuiAlert-icon": { py: 0, mr: 1, fontSize: "1.125rem" },
  "& .MuiAlert-message": { py: 0, minWidth: 0, flex: 1 },
  "& .MuiAlert-action": { py: 0, mr: -0.5, pl: 1, alignItems: "center" },
} as const;

/** Paint-only chrome; no inner compositor promotion in the Review peek.
 *  Routine states take no space. A state that needs the reader is one flat
 *  row: tint and icon only, no elevation or transform. The data attribute is
 *  always present for diagnostics and the browser conformance fixture. */
export function ReviewCodeStatus(
  { intelligence, diff }: {
    intelligence: OwnedReviewIntelligence;
    diff?: { checkFile: () => void };
  },
) {
  const { status } = intelligence;
  if (
    status === "checking" || status === "ready" || status === "incomplete"
  ) {
    return (
      <Box
        component="span"
        data-review-code-status={status}
        sx={{ display: "none" }}
      />
    );
  }
  const [severity, summary, detail] = status === "mismatch"
    ? diff
      ? [
        "warning",
        "Code intelligence differs · diagnostics hidden",
        "Current file and Code buffer differ. Open full source to review a reload. Diff never reloads the buffer.",
      ] as const
      : [
        "warning",
        "Code intelligence is out of date · diagnostics hidden",
        "Code intelligence has an older copy of this file. Diagnostics and symbols stay hidden until it matches the text shown here.",
      ] as const
    : status === "navigation"
    ? [
      "info",
      "Source check required after navigation",
      "Finish and release the original navigation in the symbol view or Settings → About, then Check this source. Nothing is reopened automatically.",
    ] as const
    : status === "synchronization"
    ? [
      "info",
      "Reload prepared · confirm in Settings → About",
      "Reload from disk is prepared, not applied. Confirm it in Settings → About → Code synchronization, then check again.",
    ] as const
    : [
      "info",
      "Code intelligence unavailable",
      "Code intelligence is unavailable for this file. Nothing was reopened or retried automatically.",
    ] as const;
  return (
    <Alert
      data-review-code-status={status}
      severity={severity}
      action={
        <>
          {status === "mismatch" && !diff && (
            <Button
              size="small"
              color="inherit"
              onClick={intelligence.prepareRefresh}
              sx={reviewCodeStatusAction}
            >
              Reload…
            </Button>
          )}
          {status === "synchronization" && !diff && (
            <Button
              size="small"
              color="inherit"
              onClick={() => openAppSettings({ tab: "info", section: "code" })}
              sx={reviewCodeStatusAction}
            >
              Confirm
            </Button>
          )}
          {status === "navigation" && (
            <Button
              size="small"
              color="inherit"
              onClick={() => openAppSettings({ tab: "info", section: "code" })}
              sx={reviewCodeStatusAction}
            >
              Navigation
            </Button>
          )}
          {status === "mismatch" && diff && (
            <Button
              size="small"
              color="inherit"
              onClick={diff.checkFile}
              sx={reviewCodeStatusAction}
            >
              Check file
            </Button>
          )}
          <Button
            size="small"
            color="inherit"
            onClick={intelligence.check}
            sx={{ ...reviewCodeStatusAction, fontWeight: 500, opacity: 0.8 }}
          >
            Check
          </Button>
        </>
      }
      sx={reviewCodeStatusRow}
    >
      <Typography variant="body2" noWrap title={detail}>
        {summary}
      </Typography>
    </Alert>
  );
}
