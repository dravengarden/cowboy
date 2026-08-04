import { MenuBookOutlined } from "@mui/icons-material";
import { Button, Tooltip } from "@mui/material";
import { ShortcutKeycap } from "../ShortcutKeycap";
import { desktopEmbeddedControlSx } from "./DesktopEmbeddedControl";

export function DesktopReadingModeControl({
  shortcutActive,
  onEnter,
}: {
  shortcutActive: boolean;
  onEnter: () => void;
}): React.JSX.Element {
  return (
    <Tooltip title="Open the current view in distraction-free Reading mode (Z)">
      <Button
        data-desktop-reading-mode-control
        aria-label="Open Reading mode"
        aria-keyshortcuts="Z"
        size="small"
        color="inherit"
        variant="outlined"
        startIcon={
          <MenuBookOutlined
            fontSize="small"
            sx={{ color: shortcutActive ? "primary.main" : "text.secondary" }}
          />
        }
        onClick={onEnter}
        sx={{
          ...desktopEmbeddedControlSx({ active: shortcutActive }),
          height: 34,
          minWidth: 0,
          px: 0.9,
          mr: 0.75,
          gap: 0.65,
          textTransform: "none",
          whiteSpace: "nowrap",
          "& .MuiButton-startIcon": { mr: 0 },
        }}
      >
        Reading
        <ShortcutKeycap
          keyLabel="Z"
          variant="global"
          accent={shortcutActive}
          availability={shortcutActive ? "available" : "inactive"}
          sx={{ ml: 0.15, flexShrink: 0 }}
        />
      </Button>
    </Tooltip>
  );
}
