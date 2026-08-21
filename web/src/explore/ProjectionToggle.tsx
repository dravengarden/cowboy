import { AutoStoriesOutlined, HistoryOutlined } from "@mui/icons-material";
import {
  Box,
  IconButton,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
} from "@mui/material";
import type { TranscriptProjection } from "./exploreStore";
import { ShortcutKeycap } from "../ShortcutKeycap";
import {
  DESKTOP_INSET_RADIUS,
  desktopEmbeddedControlSx,
} from "../desktop/DesktopEmbeddedControl";

export function MobileProjectionToggle({
  projection,
  onChange,
}: {
  projection: TranscriptProjection;
  onChange: (projection: TranscriptProjection) => void;
}): React.JSX.Element {
  const next = projection === "history" ? "explore" : "history";
  return (
    <Tooltip title={`Switch to ${next === "explore" ? "Explore" : "History"}`}>
      <IconButton
        aria-label={`Switch to ${next === "explore" ? "Explore" : "History"} view`}
        color={projection === "explore" ? "primary" : "default"}
        onClick={(): void => onChange(next)}
      >
        {projection === "history"
          ? <AutoStoriesOutlined />
          : <HistoryOutlined />}
      </IconButton>
    </Tooltip>
  );
}

export function DesktopProjectionToggle({
  projection,
  onChange,
  shortcutActive = false,
  pageLabel = "Explore",
}: {
  projection: TranscriptProjection;
  onChange: (projection: TranscriptProjection) => void;
  shortcutActive?: boolean;
  pageLabel?: string;
}): React.JSX.Element {
  return (
    <Box
      data-desktop-conversation-projection
      sx={{
        // Pane focus makes the shortcut available, but it must not make every
        // action in the Conversation rail look selected. The exclusive
        // ToggleButtonGroup below owns the History/Explore selection.
        ...desktopEmbeddedControlSx(),
        mr: 0.75,
        height: 34,
        p: "3px",
        gap: 0.25,
        display: "inline-flex",
        alignItems: "center",
        overflow: "hidden",
      }}
    >
      <ToggleButtonGroup
        exclusive
        size="small"
        value={projection}
        onChange={(_event, value: TranscriptProjection | null): void => {
          if (value) onChange(value);
        }}
        aria-label="Transcript view"
        sx={{
          height: 28,
          "& .MuiToggleButtonGroup-grouped": {
            minWidth: 62,
            px: 1.1,
            py: 0,
            border: 0,
            borderRadius: `${DESKTOP_INSET_RADIUS}px !important`,
            textTransform: "none",
            fontSize: "0.72rem",
            color: "text.secondary",
            "&.Mui-selected": {
              color: "text.primary",
              bgcolor: "action.selected",
            },
            "&.Mui-selected:hover": { bgcolor: "action.selected" },
          },
        }}
      >
        <ToggleButton value="history">History</ToggleButton>
        <ToggleButton value="explore">{pageLabel}</ToggleButton>
      </ToggleButtonGroup>
      <ShortcutKeycap
        keyLabel="V"
        variant="global"
        accent={shortcutActive}
        availability={shortcutActive ? "available" : "inactive"}
        sx={{ mx: 0.4, flexShrink: 0 }}
      />
    </Box>
  );
}
