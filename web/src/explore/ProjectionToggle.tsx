import { AutoStoriesOutlined, HistoryOutlined } from "@mui/icons-material";
import {
  IconButton,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
} from "@mui/material";
import type { TranscriptProjection } from "./exploreStore";

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
}: {
  projection: TranscriptProjection;
  onChange: (projection: TranscriptProjection) => void;
}): React.JSX.Element {
  return (
    <ToggleButtonGroup
      exclusive
      size="small"
      value={projection}
      onChange={(_event, value: TranscriptProjection | null): void => {
        if (value) onChange(value);
      }}
      aria-label="Transcript view"
      sx={{
        mr: 1,
        height: 28,
        "& .MuiToggleButton-root": {
          px: 1.1,
          py: 0,
          textTransform: "none",
          fontSize: "0.72rem",
        },
      }}
    >
      <ToggleButton value="history">History</ToggleButton>
      <ToggleButton value="explore">Explore</ToggleButton>
    </ToggleButtonGroup>
  );
}

