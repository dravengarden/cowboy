import { Box, ButtonBase, Typography } from "@mui/material";
import { activityRowSurface } from "./activityRowStyle";
import {
  Block,
  CheckCircleOutline,
  ChevronRight,
  ErrorOutline,
} from "@mui/icons-material";
import { glyphSize } from "./surface/desktopSize";
import { openBackgroundTask } from "./backgroundTaskInspectorState";
import {
  type CallNotice,
  callNoticeStateLabel,
  callNoticeTone,
} from "./callNotice";

export { pendingCallNotice } from "./callNotice";

const ICONS = {
  success: CheckCircleOutline,
  error: ErrorOutline,
  warning: Block,
} as const;

/** Compact system activity with a title and secondary status line.
 *  Embedded queue rows borrow the panel surface instead of nesting a card.
 *  A tap opens that call
 *  in the background-task inspector of `sessionId`, or of the enclosing
 *  transcript (`data-transcript-session`). */
export function CallNoticeLine({
  notice,
  sessionId,
  embedded = false,
}: {
  notice: CallNotice;
  sessionId?: string | undefined;
  embedded?: boolean;
}): React.JSX.Element {
  const tone = callNoticeTone(notice);
  const Icon = ICONS[tone];
  const title = notice.title ?? notice.callId;
  const state = callNoticeStateLabel(notice);
  const content = (
    <>
      <Icon
        aria-hidden
        sx={{ fontSize: glyphSize(16), color: `${tone}.main`, flexShrink: 0 }}
      />
      <Box component="span" sx={{ flex: 1, minWidth: 0 }}>
        <Typography
          component="span"
          variant="body2"
          noWrap
          title={title}
          sx={{
            display: "block",
            fontSize: "0.875rem",
            fontWeight: 600,
            color: "text.primary",
          }}
        >
          {title}
        </Typography>
        <Typography
          component="span"
          variant="caption"
          noWrap
          sx={{
            display: "block",
            color: "text.secondary",
            fontWeight: 400,
            mt: 0.25,
          }}
        >
          Call · {state}
        </Typography>
      </Box>
      <ChevronRight
        aria-hidden
        sx={{ fontSize: glyphSize(16), color: "text.disabled", flexShrink: 0 }}
      />
    </>
  );
  const sx = {
    ...activityRowSurface,
    ...(embedded && { border: 0, bgcolor: "transparent", borderRadius: "6px" }),
    width: "100%",
    minWidth: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "flex-start",
    gap: 1,
    minHeight: 48,
    pl: 1,
    pr: 1,
    py: 0.75,
    textAlign: "left",
  } as const;
  return (
    <ButtonBase
      data-call-notice={notice.callId}
      data-call-notice-tone={tone}
      aria-haspopup="dialog"
      aria-label={`Call ${title}: ${state}. Open details`}
      onClick={(event): void => {
        const owner = sessionId ??
          event.currentTarget.closest("[data-transcript-session]")
            ?.getAttribute("data-transcript-session");
        if (owner) {
          openBackgroundTask(
            owner,
            `call:${notice.callId}`,
            "row",
            event.currentTarget,
          );
        }
      }}
      sx={{ ...sx, "&:hover, &:focus-visible": { bgcolor: "action.hover" } }}
    >
      {content}
    </ButtonBase>
  );
}
