import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import { openAppSettings } from "../../appSettings";
import type { OwnedReviewIntelligence } from "./useOwnedReviewBuffer";

/** Paint-only chrome; no inner compositor promotion in the Review peek.
 *  Routine states stay a quiet caption. States that need the reader's decision
 *  use a flat standard Alert: tint and icon only, no elevation or transform. */
export function ReviewCodeStatus(
  { intelligence }: { intelligence: OwnedReviewIntelligence },
) {
  const { status } = intelligence;
  const button = { textTransform: "none", fontWeight: 600 } as const;
  const checkAgain = (
    <Button
      size="small"
      color="inherit"
      onClick={intelligence.check}
      sx={button}
    >
      Check again
    </Button>
  );
  if (status === "checking" || status === "ready" || status === "incomplete") {
    return (
      <Stack
        data-review-code-status={status}
        direction="row"
        alignItems="center"
        spacing={1}
        sx={{ px: 2, py: 0.5, flexShrink: 0, minHeight: 36 }}
      >
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ flex: 1, minWidth: 0 }}
        >
          {status === "checking"
            ? "Checking Code intelligence…"
            : status === "ready"
            ? "Code intelligence matches this file · diagnostics as last reported."
            : "Code intelligence needs a complete text file of at most 4 MiB."}
        </Typography>
        {status === "ready" && (
          <Button size="small" onClick={intelligence.check} sx={button}>
            Check again
          </Button>
        )}
      </Stack>
    );
  }
  const [severity, title, detail] = status === "mismatch"
    ? [
      "warning",
      "Code intelligence has an older copy of this file.",
      "Diagnostics and symbols are hidden until it matches the text shown here.",
    ] as const
    : status === "synchronization"
    ? [
      "info",
      "Reload from disk is prepared, not applied.",
      "Confirm it in Settings → About → Code synchronization, then check again.",
    ] as const
    : [
      "info",
      "Code intelligence is unavailable for this file.",
      "Nothing was reopened or retried automatically.",
    ] as const;
  return (
    <Box data-review-code-status={status} sx={{ flexShrink: 0 }}>
      <Alert
        severity={severity}
        sx={{
          borderRadius: 0,
          px: 2,
          py: 0.25,
          alignItems: "flex-start",
          "& .MuiAlert-message": { minWidth: 0, flex: 1, py: 0.75 },
          "& .MuiAlert-icon": { py: 0.875 },
        }}
      >
        <Typography variant="body2" fontWeight={600}>{title}</Typography>
        <Typography variant="caption" component="p" sx={{ opacity: 0.85 }}>
          {detail}
        </Typography>
        <Stack
          direction="row"
          useFlexGap
          flexWrap="wrap"
          spacing={1}
          sx={{ mt: 0.5, ml: -0.75 }}
        >
          {status === "mismatch" && (
            <Button
              size="small"
              color="inherit"
              variant="outlined"
              onClick={intelligence.prepareRefresh}
              sx={button}
            >
              Reload from disk…
            </Button>
          )}
          {status === "synchronization" && (
            <Button
              size="small"
              color="inherit"
              variant="outlined"
              onClick={() => openAppSettings({ tab: "info", section: "code" })}
              sx={button}
            >
              Confirm in Settings
            </Button>
          )}
          {checkAgain}
        </Stack>
      </Alert>
    </Box>
  );
}
