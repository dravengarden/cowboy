import { Box, CircularProgress, Stack, Typography } from "@mui/material";
import { frostedPill } from "./frostedGlass";
import { TURN_STATUS_PILL_MIN_HEIGHT } from "./floatingOverlayPolicy";

export function TranscriptReconnectingActivity(): React.JSX.Element {
  const label = "Reconnecting…";
  return (
    <Box
      data-transcript-turn-activity="reconnecting"
      sx={{
        display: "flex",
        justifyContent: "center",
        width: "100%",
        minWidth: 0,
        px: 2,
        boxSizing: "border-box",
        pointerEvents: "none",
      }}
    >
      <Stack
        role="status"
        aria-label={label}
        direction="row"
        alignItems="center"
        spacing={1}
        sx={(theme) => ({
          px: 2,
          py: 0.5,
          minHeight: TURN_STATUS_PILL_MIN_HEIGHT,
          borderRadius: 999,
          ...frostedPill(theme, theme.palette.warning.main),
        })}
      >
        <CircularProgress
          size={14}
          thickness={5}
          sx={{ color: "warning.main", flexShrink: 0 }}
        />
        <Typography
          variant="body2"
          sx={{ fontWeight: 600, color: "warning.main", whiteSpace: "nowrap" }}
        >
          {label}
        </Typography>
      </Stack>
    </Box>
  );
}
