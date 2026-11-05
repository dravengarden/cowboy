import { alpha, ButtonBase, type Theme, Typography } from "@mui/material";
import { Block, CheckCircleOutline, ChevronRight, ErrorOutline } from "@mui/icons-material";
import { glyphSize } from "./surface/desktopSize";
import { openBackgroundTask } from "./backgroundTaskInspectorState";
import { type CallNotice, callNoticeStateLabel, callNoticeTone } from "./callNotice";

export { pendingCallNotice } from "./callNotice";

const ICONS = { success: CheckCircleOutline, error: ErrorOutline, warning: Block } as const;

/** One-line system callback for a finished managed call. It is visibly not a
 *  person's message (no bubble, tinted rail, status glyph) and spends one row
 *  instead of the notice's three-line agent instruction. A tap opens that call
 *  in the background-task inspector of `sessionId`, or of the enclosing
 *  transcript (`data-transcript-session`). */
export function CallNoticeLine({
  notice,
  sessionId,
}: {
  notice: CallNotice;
  sessionId?: string | undefined;
}): React.JSX.Element {
  const tone = callNoticeTone(notice);
  const Icon = ICONS[tone];
  const title = notice.title ?? notice.callId;
  const state = callNoticeStateLabel(notice);
  const content = (
    <>
      <Icon aria-hidden sx={{ fontSize: glyphSize(16), color: `${tone}.main`, flexShrink: 0 }} />
      <Typography
        component="span"
        variant="caption"
        sx={{ flexShrink: 0, color: "text.secondary", fontWeight: 600, letterSpacing: "0.02em" }}
      >
        Call
      </Typography>
      <Typography
        component="span"
        variant="body2"
        noWrap
        title={title}
        sx={{ flex: 1, minWidth: 0, fontSize: "0.8125rem", fontWeight: 600, color: "text.primary" }}
      >
        {title}
      </Typography>
      <Typography
        component="span"
        variant="caption"
        noWrap
        sx={{ flexShrink: 0, maxWidth: "40%", color: `${tone}.main`, fontWeight: 600 }}
      >
        {state}
      </Typography>
      <ChevronRight aria-hidden sx={{ fontSize: glyphSize(16), color: "text.disabled", flexShrink: 0 }} />
    </>
  );
  const sx = {
    width: "100%",
    minWidth: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "flex-start",
    gap: 0.75,
    minHeight: 32,
    pl: 1,
    pr: 0.5,
    py: 0.25,
    textAlign: "left",
    borderRadius: 1,
    borderLeft: 2,
    borderColor: `${tone}.main`,
    bgcolor: (t: Theme) =>
      alpha(t.palette[tone].main, t.palette.mode === "dark" ? 0.1 : 0.05),
  } as const;
  return (
    <ButtonBase
      data-call-notice={notice.callId}
      data-call-notice-tone={tone}
      aria-haspopup="dialog"
      aria-label={`Call ${title}: ${state}. Open details`}
      onClick={(event): void => {
        const owner = sessionId ??
          event.currentTarget.closest("[data-transcript-session]")?.getAttribute("data-transcript-session");
        if (owner) openBackgroundTask(owner, `call:${notice.callId}`, "row", event.currentTarget);
      }}
      sx={{ ...sx, "&:hover, &:focus-visible": { filter: "brightness(0.97)" } }}
    >
      {content}
    </ButtonBase>
  );
}
