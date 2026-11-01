import { Close, HelpOutline, SendOutlined } from "@mui/icons-material";
import { Button, IconButton, Stack, Typography } from "@mui/material";
import { useState } from "react";
import type { ReviewLineSelection } from "./reviewLineSelection";

/** What can be done with the selected lines, where a thumb can reach it. */
export function ReviewSelectionBar({ selection, onClear, onSend, onExplain }: {
  selection: ReviewLineSelection;
  onClear: () => void;
  onSend: () => void;
  /** Ask a fork of the Agent; absent where a session cannot be forked. */
  onExplain?: (() => Promise<void>) | undefined;
}): React.JSX.Element {
  const count = selection.to - selection.from + 1;
  const [explaining, setExplaining] = useState(false);
  const [error, setError] = useState<string>();
  const explain = async (): Promise<void> => {
    setExplaining(true);
    setError(undefined);
    try {
      await onExplain?.();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not start Explain");
    } finally {
      setExplaining(false);
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
        <Button
          data-review-explain
          size="small"
          startIcon={<HelpOutline />}
          disabled={explaining}
          onClick={() => void explain()}
        >
          {explaining ? "Starting…" : "Explain"}
        </Button>
      )}
      <Button
        size="small"
        variant="contained"
        disableElevation
        startIcon={<SendOutlined />}
        onClick={onSend}
      >
        Send to Agent
      </Button>
    </Stack>
    </>
  );
}
