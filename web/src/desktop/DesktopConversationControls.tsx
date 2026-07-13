import { Box, Button, Stack, Tooltip, Typography } from "@mui/material";
import { South } from "@mui/icons-material";
import { requestStickToBottom, setSticky, useSticky } from "../stickyStore";
import { DesktopShortcut } from "./commands/DesktopKeycap";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";

const READER_HINTS = [
  { shortcut: "J/K", label: "Scroll" },
  { shortcut: "Ctrl+D/U", label: "Half page" },
  { shortcut: "Ctrl+F/B", label: "Page" },
  { shortcut: "GG/G", label: "Oldest/latest" },
] as const;

export function DesktopConversationControls(
  { sessionId }: { sessionId: string },
): React.JSX.Element {
  const following = useSticky(sessionId);
  const workspace = useDesktopWorkspace();
  const focused = workspace.focusedRegion === "conversation.transcript";
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
      {focused && (
        <Stack
          data-desktop-conversation-shortcuts
          direction="row"
          alignItems="center"
          spacing={0.75}
          sx={{
            minWidth: 0,
            mr: 0.25,
            "@media (max-width: 1240px)": {
              "& [data-conversation-hint-label]": { display: "none" },
            },
          }}
        >
          {READER_HINTS.map(({ shortcut, label }) => (
            <Tooltip
              key={shortcut}
              title={`${label} · ${shortcut}`}
              enterDelay={450}
            >
              <Box
                sx={{ display: "inline-flex", alignItems: "center", gap: 0.4 }}
              >
                <DesktopShortcut shortcut={shortcut} quiet />
                <Typography
                  data-conversation-hint-label
                  variant="caption"
                  color="text.secondary"
                  sx={{ whiteSpace: "nowrap", fontSize: "0.65rem" }}
                >
                  {label}
                </Typography>
              </Box>
            </Tooltip>
          ))}
        </Stack>
      )}
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
          endIcon={focused ? <DesktopShortcut shortcut="F" quiet /> : undefined}
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
            "& .MuiButton-endIcon": { ml: 0.65 },
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
