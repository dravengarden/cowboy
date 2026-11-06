import { KeyboardArrowDown, KeyboardArrowUp } from "@mui/icons-material";
import { Box, Button, Chip, CircularProgress, IconButton, Stack, Typography } from "@mui/material";

/** Presentation only; DocumentView owns paging and the selected hunk. */
export function ReviewDocumentControls({
  counts,
  hunkCount,
  hunkIndex,
  onPrevious,
  onNext,
  hasMore,
  loadingMore,
  loadMoreError,
  onLoadMore,
  limited,
  diff,
  children,
}: {
  counts: { added: number; removed: number } | undefined;
  hunkCount: number;
  hunkIndex: number;
  onPrevious: () => void;
  onNext: () => void;
  hasMore: boolean;
  loadingMore: boolean;
  loadMoreError: boolean;
  onLoadMore: () => void;
  limited: boolean;
  diff: boolean;
  children?: React.ReactNode;
}): React.JSX.Element {
  return (
    <Stack
      data-review-document-controls
      direction="row"
      spacing={1}
      alignItems="center"
      sx={{
        px: 1.5,
        flexShrink: 0,
        minHeight: 48,
        flexWrap: "wrap",
        borderTop: 1,
        borderColor: "divider",
        "&:empty": { display: "none" },
        "& .MuiIconButton-root, & .MuiButton-root": { minWidth: 44, minHeight: 44 },
      }}
    >
      {children}
      {counts && (
        <>
          <Typography variant="caption" color="success.main">
            +{counts.added}
          </Typography>
          <Typography variant="caption" color="error.main">
            −{counts.removed}
          </Typography>
        </>
      )}
      {hunkCount > 0 && (
        <>
          <Box sx={{ flex: 1 }} />
          <IconButton
            size="small"
            aria-label="Previous hunk"
            disabled={hunkIndex <= 0}
            onClick={onPrevious}
          >
            <KeyboardArrowUp fontSize="small" />
          </IconButton>
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ minWidth: 36, textAlign: "center" }}
          >
            {hunkIndex + 1}/{hunkCount}
          </Typography>
          <IconButton
            size="small"
            aria-label="Next hunk"
            disabled={hunkIndex >= hunkCount - 1}
            onClick={onNext}
          >
            <KeyboardArrowDown fontSize="small" />
          </IconButton>
        </>
      )}
      {hasMore && (
        <Button
          size="small"
          variant="outlined"
          disabled={loadingMore}
          onClick={onLoadMore}
          startIcon={loadingMore ? <CircularProgress size={12} /> : undefined}
        >
          {loadMoreError ? "Retry" : "Load more"}
        </Button>
      )}
      {limited && !hasMore && (
        <Chip
          size="small"
          color="warning"
          label={diff ? "Preview limited to 16 MB" : "Preview limited to 32 MB"}
        />
      )}
    </Stack>
  );
}
