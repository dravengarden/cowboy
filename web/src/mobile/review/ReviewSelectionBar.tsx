import { Close } from "@mui/icons-material";
import { Button, IconButton, Stack, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import type { ReviewLineSelection, ReviewQuestion } from "../../review/reviewLineSelection";

/** What can be done with the selected lines, where a thumb can reach it. */
export function ReviewSelectionBar({ selection, onClear, onSend, onExplain }: {
  selection: ReviewLineSelection;
  onClear: () => void;
  onSend: () => void;
  /** Ask a fork of the Agent; absent where a session cannot be forked. */
  onExplain?: ((kind: ReviewQuestion) => Promise<void>) | undefined;
}): React.JSX.Element {
  const count = selection.to - selection.from + 1;
  const [asking, setAsking] = useState<ReviewQuestion>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    setError(undefined);
  }, [selection.from, selection.to]);
  const ask = async (kind: ReviewQuestion): Promise<void> => {
    setAsking(kind);
    setError(undefined);
    try {
      await onExplain?.(kind);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not start Explain");
    } finally {
      setAsking(undefined);
    }
  };
  return (
    <>
    {error && (
      <Typography
        role="alert"
        variant="caption"
        color="error.main"
        sx={{ flexShrink: 0, px: 1.5, pt: 0.5 }}
      >
        {error}
      </Typography>
    )}
    <Stack
      data-review-selection-bar
      role="toolbar"
      aria-label="Selected lines"
      direction="row"
      alignItems="center"
      spacing={0.5}
      sx={{
        flexShrink: 0,
        px: 0.75,
        minHeight: 48,
        borderTop: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
      }}
    >
      <IconButton
        aria-label="Clear selection"
        onClick={onClear}
        sx={{ width: 44, height: 44 }}
      >
        <Close fontSize="small" />
      </IconButton>
      <Typography variant="body2" noWrap sx={{ flex: 1, minWidth: 0 }}>
        {count === 1 ? "1 line" : `${count} lines`}
      </Typography>
      {onExplain && (
        <>
          <Button
            data-review-explain
            disabled={asking !== undefined}
            onClick={() => void ask("explain")}
            sx={{ minHeight: 44, px: 1.25, textTransform: "none" }}
          >
            {asking === "explain" ? "Starting…" : "Explain"}
          </Button>
          <Button
            data-review-ask-review
            disabled={asking !== undefined}
            onClick={() => void ask("review")}
            sx={{ minHeight: 44, px: 1.25, textTransform: "none" }}
          >
            {asking === "review" ? "Starting…" : "Review"}
          </Button>
        </>
      )}
      <Button
        variant="contained"
        disableElevation
        aria-label="Send to Agent"
        onClick={onSend}
        sx={{ minHeight: 44, px: 1.75, textTransform: "none" }}
      >
        Send
      </Button>
    </Stack>
    </>
  );
}
