import { Close } from "@mui/icons-material";
import { Button, IconButton, Stack, Typography } from "@mui/material";
import { useState } from "react";
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
      spacing={1}
      sx={{
        flexShrink: 0,
        px: 1,
        minHeight: 44,
        borderTop: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
      }}
    >
      <IconButton
        size="small"
        aria-label="Clear selection"
        onClick={onClear}
      >
        <Close fontSize="small" />
      </IconButton>
      <Typography variant="body2" sx={{ flex: 1 }}>
        {count === 1 ? "1 line" : `${count} lines`}
      </Typography>
      {onExplain && (
        <>
          <Button
            data-review-explain
            size="small"
            disabled={asking !== undefined}
            onClick={() => void ask("explain")}
          >
            {asking === "explain" ? "Starting…" : "Explain"}
          </Button>
          <Button
            data-review-ask-review
            size="small"
            disabled={asking !== undefined}
            onClick={() => void ask("review")}
          >
            {asking === "review" ? "Starting…" : "Review"}
          </Button>
        </>
      )}
      <Button
        size="small"
        variant="contained"
        disableElevation
        aria-label="Send to Agent"
        onClick={onSend}
      >
        Send
      </Button>
    </Stack>
    </>
  );
}
