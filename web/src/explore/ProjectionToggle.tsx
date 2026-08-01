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
import { desktopEmbeddedControlSx } from "../desktop/DesktopEmbeddedControl";

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
}: {
  projection: TranscriptProjection;
  onChange: (projection: TranscriptProjection) => void;
  shortcutActive?: boolean;
}): React.JSX.Element {
  return (
    <Box
      data-desktop-conversation-projection
      sx={{
        ...desktopEmbeddedControlSx({ active: shortcutActive }),
        mr: 0.75,
        height: 30,
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
          height: "100%",
          "& .MuiToggleButtonGroup-grouped": {
            minWidth: 58,
            px: 1.1,
            py: 0,
            border: 0,
            borderRadius: "999px !important",
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
        <ToggleButton value="explore">Explore</ToggleButton>
      </ToggleButtonGroup>
      <ShortcutKeycap
        keyLabel="V"
        variant="global"
        accent={shortcutActive}
        sx={{ mx: 0.65, flexShrink: 0 }}
      />
    </Box>
  );
}
