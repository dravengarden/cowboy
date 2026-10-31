import { Close, SendOutlined } from "@mui/icons-material";
import { Button, IconButton, Stack, Typography } from "@mui/material";
import type { ReviewLineSelection } from "./reviewLineSelection";

/** What can be done with the selected lines, where a thumb can reach it. */
export function ReviewSelectionBar({ selection, onClear, onSend }: {
  selection: ReviewLineSelection;
  onClear: () => void;
  onSend: () => void;
}): React.JSX.Element {
  const count = selection.to - selection.from + 1;
  return (
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
  );
}
