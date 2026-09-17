import { Alert, Box, Button, Typography } from "@mui/material";
import { openAppSettings } from "../../appSettings";
import type { OwnedReviewIntelligence } from "./useOwnedReviewBuffer";

const action = {
  textTransform: "none",
  fontWeight: 600,
  minWidth: 0,
  px: 1,
  whiteSpace: "nowrap",
} as const;

/** Paint-only chrome; no inner compositor promotion in the Review peek.
 *  Routine states take no space. A state that needs the reader is one flat
 *  row: tint and icon only, no elevation or transform. The data attribute is
 *  always present for diagnostics and the browser conformance fixture. */
export function ReviewCodeStatus(
  { intelligence }: { intelligence: OwnedReviewIntelligence },
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
    ? [
      "warning",
      "Code intelligence is out of date · diagnostics hidden",
      "Code intelligence has an older copy of this file. Diagnostics and symbols stay hidden until it matches the text shown here.",
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
          {status === "mismatch" && (
            <Button
              size="small"
              color="inherit"
              onClick={intelligence.prepareRefresh}
              sx={action}
            >
              Reload…
            </Button>
          )}
          {status === "synchronization" && (
            <Button
              size="small"
              color="inherit"
              onClick={() => openAppSettings({ tab: "info", section: "code" })}
              sx={action}
            >
              Confirm
            </Button>
          )}
          <Button
            size="small"
            color="inherit"
            onClick={intelligence.check}
            sx={{ ...action, fontWeight: 500, opacity: 0.8 }}
          >
            Check
          </Button>
        </>
      }
      sx={{
        flexShrink: 0,
        borderRadius: 0,
        minHeight: 40,
        px: 1.5,
        py: 0,
        alignItems: "center",
        "& .MuiAlert-icon": { py: 0, mr: 1, fontSize: "1.125rem" },
        "& .MuiAlert-message": { py: 0, minWidth: 0, flex: 1 },
        "& .MuiAlert-action": { py: 0, mr: -0.5, pl: 1, alignItems: "center" },
      }}
    >
      <Typography variant="body2" noWrap title={detail}>
        {summary}
      </Typography>
    </Alert>
  );
}
