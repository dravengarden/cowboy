import { Button, Stack, Tooltip } from "@mui/material";
import { South } from "@mui/icons-material";
import { requestStickToBottom, setSticky, useSticky } from "../stickyStore";

export function DesktopConversationControls(
  { sessionId }: { sessionId: string },
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
    <Stack
      direction="row"
      alignItems="center"
      spacing={0.75}
      sx={{ minWidth: 0 }}
    >
      <Tooltip
        title={following
          ? "Pause automatic transcript following (F)"
          : "Jump to the latest output and follow (F)"}
      >
        <Button
          data-desktop-conversation-follow
          size="small"
          color={following ? "primary" : "inherit"}
          variant={following ? "contained" : "text"}
          startIcon={<South fontSize="small" />}
          onClick={toggle}
          sx={{
            height: 28,
            minWidth: 0,
            px: 1,
            mr: 0.5,
            borderRadius: 1.25,
            textTransform: "none",
            boxShadow: "none",
            whiteSpace: "nowrap",
            ...(following && {
              bgcolor: "action.selected",
              color: "primary.main",
              "&:hover": { bgcolor: "action.selected", boxShadow: "none" },
            }),
          }}
        >
          {following ? "Following" : "Follow"}
        </Button>
      </Tooltip>
    </Stack>
  );
}
