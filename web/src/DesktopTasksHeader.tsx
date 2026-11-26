// The Desktop Background tasks dialog's title row: what it lists, Locate for
// a task anchored in the transcript, and Close.
import { Box, Button, Stack, Typography } from "@mui/material";
import { KeyboardArrowDown, MyLocation } from "@mui/icons-material";

export function DesktopTasksHeader({
  onLocate,
  onClose,
}: {
  /** Scroll the transcript to the selected task, when it is anchored there. */
  onLocate: (() => void) | undefined;
  onClose: () => void;
}): React.JSX.Element {
  return (
    <Box
      sx={{
        px: 2,
        py: 1,
        display: "grid",
        gridTemplateColumns: "clamp(240px, 18vw, 320px) minmax(0, 1fr)",
        alignItems: "center",
        gap: 2,
        borderBottom: 1,
        borderColor: "divider",
      }}
    >
      <Box>
        <Typography id="background-tasks-title" variant="subtitle1" sx={{ fontWeight: 800 }}>
          Background tasks
        </Typography>
        <Typography variant="caption" color="text.secondary">
          Subagents and calls this session started
        </Typography>
      </Box>
      <Stack
        direction="row"
        alignItems="center"
        spacing={1.25}
        sx={{ minWidth: 0, justifyContent: "flex-end" }}
      >
        {onLocate && (
          <Button
            onClick={onLocate}
            startIcon={<MyLocation fontSize="small" />}
            aria-label="Locate task in transcript"
            sx={{ textTransform: "none" }}
          >
            Locate
          </Button>
        )}
        <Button
          onClick={onClose}
          endIcon={<KeyboardArrowDown />}
          sx={{ textTransform: "none", flexShrink: 0 }}
        >
          Close
        </Button>
      </Stack>
    </Box>
  );
}
