import { Box, Button, Tooltip } from "@mui/material";
import { alpha } from "@mui/material/styles";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import type { RenderItem } from "./derive";
import { requestNewFrom } from "./sessionSettingsOpen";
import { useStoreSelector } from "./store";
import { glyphSize } from "./surface/desktopSize";
import type { TurnForkPoint } from "./turnForkPoints";

/** Zero-height Desktop hover affordance straddling the bottom-right corner of
 *  a completed turn-final assistant response. Opens Replace or fork already
 *  targeted at this turn so the reader can confirm and pick options without
 *  cluttering the transcript or risking accidental one-click forks. */
export function MessageForkAction({
  originSessionId,
  forkPoint,
}: {
  originSessionId: string;
  forkPoint: TurnForkPoint;
}): React.JSX.Element | null {
  const forkEligible = useStoreSelector((state) => {
    const session = state.sessions.find((entry) => entry.id === originSessionId);
    return Boolean(session && !session.system && session.workspace_id);
  });
  if (!forkEligible) return null;

  return (
    <Box
      data-message-fork-bar
      sx={{
        position: "absolute",
        right: 10,
        bottom: -11,
        zIndex: 1,
        visibility: "hidden",
        opacity: 0,
        pointerEvents: "none",
        ".cowboy-msg-wrap:hover &, .cowboy-msg-wrap:focus-within &": {
          visibility: "visible",
          opacity: 1,
          pointerEvents: "auto",
        },
      }}
    >
      <Tooltip title={`Fork from Turn ${String(forkPoint.turnNumber)}`}>
        <Button
          size="small"
          variant="outlined"
          color="inherit"
          data-fork-from-here
          data-desktop-transcript-stop="action"
          startIcon={<CallSplitOutlined sx={{ fontSize: glyphSize(13) }} />}
          onClick={(): void => {
            requestNewFrom(originSessionId, {
              itemKey: forkPoint.itemKey,
              messageId: forkPoint.messageId,
              beforeSeq: forkPoint.beforeSeq,
              turnNumber: forkPoint.turnNumber,
              promptPreview: forkPoint.promptPreview,
            });
          }}
          sx={(theme) => ({
            minWidth: 0,
            minHeight: 22,
            px: 0.85,
            py: 0,
            borderRadius: "999px",
            fontSize: "0.7rem",
            lineHeight: 1.2,
            color: "text.secondary",
            bgcolor: theme.palette.background.paper,
            borderColor: alpha(theme.palette.divider, 0.9),
            boxShadow: theme.shadows[1],
            textTransform: "none",
            "& .MuiButton-startIcon": { mr: 0.4 },
            "&:hover": {
              color: "primary.main",
              borderColor: alpha(theme.palette.primary.main, 0.45),
              bgcolor: theme.palette.background.paper,
            },
          })}
        >
          Fork from here
        </Button>
      </Tooltip>
    </Box>
  );
}

/** Wraps a completed turn-final assistant MessageBubble with a zero-height
 *  Desktop hover trigger. Mobile keeps the reading transcript completely free
 *  of inline fork chrome — turn-point forking lives in Replace or fork. */
export function MessageWithFork({
  item,
  sessionId,
  streaming,
  desktop,
  forkPoint,
  children,
}: {
  item: Extract<RenderItem, { kind: "message" }>;
  sessionId: string;
  streaming: boolean;
  desktop: boolean;
  forkPoint?: TurnForkPoint | undefined;
  children: React.JSX.Element;
}): React.JSX.Element {
  if (!desktop || item.role !== "assistant" || streaming || !forkPoint) return children;
  return (
    <Box className="cowboy-msg-wrap" sx={{ position: "relative", width: "100%", maxWidth: "100%" }}>
      {children}
      <MessageForkAction originSessionId={sessionId} forkPoint={forkPoint} />
    </Box>
  );
}
