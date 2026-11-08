import { CloudDownloadOutlined, DoneAll, VisibilityOffOutlined } from "@mui/icons-material";
import { Box, Button, ButtonBase, CircularProgress, Stack, Typography } from "@mui/material";
import { createPortal } from "react-dom";
import { useNetworkActionState } from "../../NetworkActionFeedback";
import { COARSE_POINTER_ROOT_CLASS } from "../../platform";
import { ReviewComparisonButton } from "./ReviewComparisonButton";
import { reviewFetchAge } from "./reviewCompareModel";

/** The changes view owns these controls even when the repository lays them
 * beside its tabs. Opening secondary actions never resets the comparison. */
export function ReviewChangeControls({
  container,
  actionsContainer,
  label,
  disabled,
  onCompare,
  hasChanges,
  unreadCount,
  unreadOnly,
  onUnread,
  fetchAvailable,
  onFetch,
  fetchError,
  fetchedAtMs,
}: {
  container?: HTMLElement | null | undefined;
  actionsContainer?: HTMLElement | null | undefined;
  label: string;
  disabled: boolean;
  onCompare: () => void;
  hasChanges: boolean;
  unreadCount: number;
  unreadOnly: boolean;
  onUnread: () => void;
  fetchAvailable: boolean;
  onFetch: () => Promise<void>;
  fetchError?: string | undefined;
  fetchedAtMs?: number | undefined;
}): React.JSX.Element {
  const fetch = useNetworkActionState();
  const primary = (
    <Stack
      data-review-change-controls
      direction="row"
      alignItems="center"
      sx={{ minWidth: hasChanges ? 144 : 100, flex: "1 1 100px", justifyContent: "flex-end", gap: 0.25 }}
    >
      <ReviewComparisonButton label={label} disabled={disabled} onClick={onCompare} />
      {hasChanges && (
        <ButtonBase
          data-review-unread
          aria-label={unreadCount === 0 ? "All viewed" : `${unreadCount} unread`}
          aria-pressed={unreadOnly}
          onClick={onUnread}
          sx={{
            minWidth: 48,
            minHeight: 48,
            px: 0.75,
            gap: 0.5,
            borderRadius: 1,
            color: unreadOnly ? "primary.main" : "text.secondary",
            "&&[aria-pressed='true']": { bgcolor: "action.selected" },
            [`html.${COARSE_POINTER_ROOT_CLASS} &&[aria-pressed='true'], html.${COARSE_POINTER_ROOT_CLASS} &&[aria-pressed='true']:hover`]:
              { bgcolor: "action.selected" },
          }}
        >
          {unreadCount === 0 ? (
            <DoneAll sx={{ fontSize: "1rem" }} />
          ) : (
            <VisibilityOffOutlined sx={{ fontSize: "1rem" }} />
          )}
          <Typography variant="caption" component="span">
            {unreadCount}
          </Typography>
        </ButtonBase>
      )}
    </Stack>
  );
  const secondary = (
    <Box>
      {fetchAvailable && (
        <Button
          data-review-fetch
          aria-label="Fetch this branch"
          aria-busy={fetch.pending || undefined}
          disabled={fetch.pending}
          onClick={() => void fetch.run(onFetch)}
          startIcon={
            fetch.progress ? (
              <CircularProgress size={20} color="inherit" />
            ) : (
              <CloudDownloadOutlined fontSize="small" />
            )
          }
          sx={{ minHeight: 44, color: "text.secondary", textTransform: "none" }}
        >
          Fetch this branch
        </Button>
      )}
      {(fetchError || fetchedAtMs !== undefined) && (
        <Typography
          role={fetchError ? "alert" : "status"}
          variant="caption"
          color={fetchError ? "error.main" : "text.secondary"}
          sx={{ display: "block", px: 1 }}
        >
          {fetchError ?? reviewFetchAge(fetchedAtMs!, Date.now())}
        </Typography>
      )}
    </Box>
  );
  return (
    <>
      {container === undefined ? (
        <Box sx={{ px: 1, flexShrink: 0, borderTop: 1, borderColor: "divider" }}>
          {primary}
          {secondary}
        </Box>
      ) : (
        container && createPortal(primary, container)
      )}
      {container !== undefined && actionsContainer && createPortal(secondary, actionsContainer)}
      {container !== undefined && actionsContainer === undefined && secondary}
      {container !== undefined && fetchError && actionsContainer === null && (
        <Typography role="alert" variant="caption" color="error.main" sx={{ px: 1 }}>
          {fetchError}
        </Typography>
      )}
    </>
  );
}
