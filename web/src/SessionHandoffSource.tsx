import { Box, Button, ButtonBase, IconButton, Tooltip, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import LinkOutlined from "@mui/icons-material/LinkOutlined";
import LinkOffOutlined from "@mui/icons-material/LinkOffOutlined";
import ChevronRightRounded from "@mui/icons-material/ChevronRightRounded";
import VisibilityOutlined from "@mui/icons-material/VisibilityOutlined";
import { useState } from "react";
import { glyphSize } from "./surface/desktopSize";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { sessionAccountIdentity } from "./accountIdentity";
import { restoreItems, useStoreSelector } from "./store";
import { SESSION_LINK_LABEL, type SessionLinkRow } from "./sessionLinks";
import { requestPickSession } from "./sessionPickRequest";
import { requestSessionReveal } from "./sessionReveal";
import { SessionHistoryPeek } from "./SessionHistoryPeek";
import type { SessionMeta } from "./protocol";

export function openSessionLink(link: SessionLinkRow, session: SessionMeta | undefined): void {
  if (link.before === undefined) requestPickSession(link.id, session);
  else requestSessionReveal(link.id, session, link.before);
}

export function useRestoreSession(id: string): {
  busy: boolean;
  failure: string;
  restore: () => void;
} {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState("");
  const restore = (): void => {
    setBusy(true);
    setFailure("");
    void restoreItems({ sessions: [id] })
      .catch((error: unknown) =>
        setFailure(error instanceof Error ? error.message : "Could not restore"),
      )
      .finally(() => setBusy(false));
  };
  return { busy, failure, restore };
}

function HandoffCardBody({
  link,
  label,
  failure,
  peekable,
  open,
}: {
  link: SessionLinkRow;
  label: string;
  failure: string;
  peekable: boolean;
  open: boolean;
}): React.JSX.Element {
  const Icon =
    link.state === "deleted"
      ? LinkOffOutlined
      : link.kind === "forked"
        ? CallSplitOutlined
        : LinkOutlined;
  return (
    <>
      <Box
        aria-hidden
        sx={{
          display: "grid",
          placeItems: "center",
          width: "1.75rem",
          height: "1.75rem",
          flexShrink: 0,
          borderRadius: "50%",
          color: peekable ? "primary.main" : "text.secondary",
          bgcolor: (theme) =>
            peekable
              ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.2 : 0.1)
              : theme.palette.action.hover,
        }}
      >
        <Icon sx={{ fontSize: glyphSize(16) }} />
      </Box>
      <Box sx={{ flex: 1, minWidth: 0, textAlign: "left" }}>
        <Typography
          variant="caption"
          noWrap
          color={failure ? "error" : "text.secondary"}
          sx={{ display: "block", lineHeight: 1.3 }}
        >
          {failure || label}
        </Typography>
        <Typography
          variant="body2"
          noWrap
          title={link.title}
          color={peekable ? "text.primary" : "text.secondary"}
          sx={{ fontWeight: 500, lineHeight: 1.35 }}
        >
          {link.title}
        </Typography>
      </Box>
      {open && (
        <ChevronRightRounded
          sx={{ fontSize: glyphSize(18), flexShrink: 0, color: "text.secondary" }}
        />
      )}
    </>
  );
}

const FRAME_SX = {
  display: "flex",
  alignItems: "center",
  gap: 0.75,
  width: "fit-content",
  maxWidth: "min(100%, 26rem)",
  minWidth: 0,
  minHeight: 44,
  boxSizing: "border-box",
  pl: 0.75,
  pr: 0.75,
  py: 0.5,
  border: 1,
  borderColor: "divider",
  borderRadius: 3,
  bgcolor: "background.paper",
} as const;

const TRIGGER_SX = {
  flex: 1,
  minWidth: 0,
  minHeight: 34,
  display: "flex",
  alignItems: "center",
  gap: 1.1,
  px: 0.5,
  borderRadius: 2,
  "&:hover": { bgcolor: "action.hover" },
} as const;

/** Where a continuation or fork came from, as one compact provenance card:
 *  live sources open at the handoff point or preview in place; trashed
 *  sources open read-only without forcing Restore; deleted ones stay named. */
export function SessionHandoffSource({ link }: { link: SessionLinkRow }): React.JSX.Element {
  const session = useStoreSelector((state) => state.sessions.find((entry) => entry.id === link.id));
  const trashed = useStoreSelector((state) => state.trashedSessions.find((entry) => entry.id === link.id));
  const meta = session ?? trashed;
  const { data: accounts } = useSubscriptionAccounts();
  const { busy, failure, restore } = useRestoreSession(link.id);
  const [peekOpen, setPeekOpen] = useState(false);
  const open = link.state === "open";
  const peekable = link.state === "open" || link.state === "trash";
  const note = link.state === "trash" ? "In Trash" : link.state === "deleted" ? "Deleted" : "";
  const label = [
    SESSION_LINK_LABEL[link.kind],
    meta && sessionAccountIdentity(accounts, meta).short,
    note,
  ]
    .filter(Boolean)
    .join(" · ");
  const body = <HandoffCardBody link={link} label={label} failure={failure} peekable={peekable} open={open} />;

  return (
    <>
      <Box data-session-handoff-link={link.state} sx={FRAME_SX}>
        {open ? (
          <ButtonBase
            title="Open at the point it was handed off"
            onClick={(): void => openSessionLink(link, session)}
            sx={TRIGGER_SX}
          >
            {body}
          </ButtonBase>
        ) : link.state === "trash" ? (
          <ButtonBase
            data-session-handoff-peek
            title="Preview previous conversation without restoring"
            onClick={(): void => setPeekOpen(true)}
            sx={TRIGGER_SX}
          >
            {body}
          </ButtonBase>
        ) : (
          <Box title={link.title} sx={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 1.1, px: 0.5 }}>
            {body}
          </Box>
        )}
        {open && (
          <Tooltip title="Preview previous conversation in place">
            <IconButton
              size="small"
              data-session-handoff-peek
              aria-label="Preview previous conversation"
              onClick={(): void => setPeekOpen(true)}
              sx={{ flexShrink: 0, color: "text.secondary" }}
            >
              <VisibilityOutlined sx={{ fontSize: glyphSize(17) }} />
            </IconButton>
          </Tooltip>
        )}
        {link.state === "trash" && (
          <Button
            size="small"
            variant="outlined"
            disabled={busy}
            onClick={restore}
            sx={{ flexShrink: 0, minWidth: 0, px: 1.25, borderRadius: 999, textTransform: "none" }}
          >
            {busy ? "Restoring…" : "Restore"}
          </Button>
        )}
      </Box>
      {peekable && peekOpen && (
        <SessionHistoryPeek
          open={peekOpen}
          onClose={(): void => setPeekOpen(false)}
          link={link}
          session={session}
          onOpenSession={open ? (): void => openSessionLink(link, session) : undefined}
          onRestore={link.state === "trash" ? restore : undefined}
          restoring={busy}
        />
      )}
    </>
  );
}
