import { alpha, Button, Tooltip } from "@mui/material";
import { South } from "@mui/icons-material";
import { requestStickToBottom, setSticky, useSticky } from "../stickyStore";
import { ShortcutKeycap } from "../ShortcutKeycap";

export function DesktopConversationControls(
  { sessionId, shortcutActive = false }: {
    sessionId: string;
    shortcutActive?: boolean;
  },
): React.JSX.Element {
  const following = useSticky(sessionId);
  const toggle = (): void => {
    const scroller = document.querySelector<HTMLElement>(
      "[data-desktop-transcript-scroller]",
    );
    if (scroller) {
      scroller.dispatchEvent(
        new CustomEvent("cowboy:desktop-transcript-nav", {
          detail: { action: "toggle-following" },
        }),
      );
    } else if (following) setSticky(sessionId, false);
    else requestStickToBottom(sessionId);
  };

  return (
    <Tooltip
      title={following
        ? "Pause automatic transcript following (F)"
        : "Jump to the latest output and follow (F)"}
    >
      <Button
        data-desktop-conversation-follow
        size="small"
        color={following ? "primary" : "inherit"}
        variant="outlined"
        startIcon={<South fontSize="small" />}
        onClick={toggle}
        sx={{
          height: 30,
          minWidth: 0,
          px: 0.75,
          mr: 0.75,
          gap: 0.65,
          borderRadius: 999,
          borderColor: (theme) => alpha(theme.palette.divider, 0.78),
          bgcolor: (theme) => alpha(theme.palette.background.paper, 0.34),
          textTransform: "none",
          boxShadow: "none",
          whiteSpace: "nowrap",
          "& .MuiButton-startIcon": { mr: 0 },
          ...(following && {
            bgcolor: "action.selected",
            color: "primary.main",
            "&:hover": { bgcolor: "action.selected", boxShadow: "none" },
          }),
        }}
      >
        {following ? "Following" : "Follow"}
        <ShortcutKeycap
          keyLabel="F"
          variant="global"
          accent={shortcutActive}
          sx={{ ml: 0.15, flexShrink: 0 }}
        />
      </Button>
    </Tooltip>
  );
}
