import { Box, Button, Typography } from "@mui/material";
import { openAppSettings } from "../../appSettings";
import type { OwnedReviewIntelligence } from "./useOwnedReviewBuffer";

/** Paint-only chrome; no inner compositor promotion in the Review peek. */
export function ReviewCodeStatus(
  { intelligence }: { intelligence: OwnedReviewIntelligence },
) {
  const { status } = intelligence;
  return (
    <Box
      data-review-code-status={status}
      sx={{ px: 2, py: 0.75, flexShrink: 0 }}
    >
      <Typography variant="caption" color="text.secondary">
        {status === "checking"
          ? "Checking matching Code content…"
          : status === "ready"
          ? "Content matched · diagnostics are last observed."
          : status === "mismatch"
          ? "File and Code buffer differ. No stale positions are shown."
          : status === "incomplete"
          ? "Code intelligence requires a complete text file of at most 4 MiB."
          : status === "synchronization"
          ? "Refresh prepared. Confirm or retire it in Settings → About → Code synchronization, then check again."
          : "Code intelligence is unavailable. No automatic reopen or retry was attempted."}
      </Typography>
      {status !== "checking" && status !== "incomplete" && (
        <Button size="small" onClick={intelligence.check}>Check Code</Button>
      )}
      {status === "mismatch" && (
        <Button size="small" onClick={intelligence.prepareRefresh}>
          Review refresh…
        </Button>
      )}
      {status === "synchronization" && (
        <Button
          size="small"
          onClick={() => openAppSettings({ tab: "info", section: "code" })}
        >
          Open Code synchronization
        </Button>
      )}
    </Box>
  );
}
