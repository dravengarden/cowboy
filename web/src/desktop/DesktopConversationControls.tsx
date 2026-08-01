import { Button, Tooltip } from "@mui/material";
import { South } from "@mui/icons-material";
import { requestStickToBottom, setSticky, useSticky } from "../stickyStore";
import { ShortcutKeycap } from "../ShortcutKeycap";
import {
  followExploreTail,
  type TranscriptProjection,
  useExploreAtTail,
} from "../explore/exploreStore";
import { desktopEmbeddedControlSx } from "./DesktopEmbeddedControl";

export function DesktopConversationControls(
  { sessionId, projection = "history", shortcutActive = false }: {
    sessionId: string;
    projection?: TranscriptProjection;
    shortcutActive?: boolean;
  },
): React.JSX.Element {
  const following = useSticky(sessionId);
  const exploreAtTail = useExploreAtTail(sessionId);
  const followingCurrentView = following &&
    (projection === "history" || exploreAtTail);
  const toggle = (): void => {
    if (projection === "explore" && !followingCurrentView) {
      followExploreTail(sessionId);
      requestStickToBottom(sessionId);
      return;
    }
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
      title={followingCurrentView
        ? "Pause automatic transcript following (F)"
        : "Jump to the latest output and follow (F)"}
    >
      <Button
        data-desktop-conversation-follow
        size="small"
        color={followingCurrentView ? "primary" : "inherit"}
        variant="outlined"
        startIcon={<South fontSize="small" />}
        onClick={toggle}
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
          ...(followingCurrentView && {
            bgcolor: "action.selected",
            color: "primary.main",
            "&:hover": { bgcolor: "action.selected", boxShadow: "none" },
          }),
        }}
      >
        {followingCurrentView ? "Following" : "Follow"}
        <ShortcutKeycap
          keyLabel="F"
          variant="global"
          accent={shortcutActive}
          availability={shortcutActive ? "available" : "inactive"}
          sx={{ ml: 0.15, flexShrink: 0 }}
        />
      </Button>
    </Tooltip>
  );
}
