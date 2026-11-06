import { CompareArrowsRounded, FolderOutlined } from "@mui/icons-material";
import { ToggleButton, ToggleButtonGroup } from "@mui/material";
import type { ReviewMode } from "./reviewMode";

export function ReviewModeSwitcher({
  mode,
  onChange,
}: {
  mode: ReviewMode;
  onChange: (mode: ReviewMode) => void;
}): React.JSX.Element {
  return (
    <ToggleButtonGroup
      data-mobile-review-mode-switcher
      aria-label="Switch between Git changes and Worktree files"
      exclusive
      color="primary"
      value={mode}
      onChange={(_, value: ReviewMode | null) => {
        if (value !== null) onChange(value);
      }}
      sx={{
        flex: "0 0 auto",
        "& .MuiToggleButton-root": {
          minWidth: 48,
          minHeight: 48,
          p: 1,
        },
        "& .MuiSvgIcon-root": { fontSize: "1.5rem" },
      }}
    >
      <ToggleButton value="git" aria-label="Git changes">
        <CompareArrowsRounded />
      </ToggleButton>
      <ToggleButton value="files" aria-label="Worktree files">
        <FolderOutlined />
      </ToggleButton>
    </ToggleButtonGroup>
  );
}
