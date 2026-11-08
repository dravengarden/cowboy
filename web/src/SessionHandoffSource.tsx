import { Box, Button, ButtonBase, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";
import LinkOutlined from "@mui/icons-material/LinkOutlined";
import LinkOffOutlined from "@mui/icons-material/LinkOffOutlined";
import ChevronRightRounded from "@mui/icons-material/ChevronRightRounded";
import { useState } from "react";
import { glyphSize } from "./surface/desktopSize";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { sessionAccountIdentity } from "./accountIdentity";
import { restoreItems, useStoreSelector } from "./store";
import { SESSION_LINK_LABEL, type SessionLinkRow } from "./sessionLinks";
import { requestPickSession } from "./sessionPickRequest";
import { requestSessionReveal } from "./sessionReveal";
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

/** Where a continuation came from, as one compact card above its first
 *  message: the label and state on top, the source title below, and the one
 *  thing you can do with it (open or restore) at the end. */
export function SessionHandoffSource({ link }: { link: SessionLinkRow }): React.JSX.Element {
  const session = useStoreSelector((state) => state.sessions.find((entry) => entry.id === link.id));
  const { data: accounts } = useSubscriptionAccounts();
  const { busy, failure, restore } = useRestoreSession(link.id);
  const open = link.state === "open";
  const note = link.state === "trash" ? "In Trash" : link.state === "deleted" ? "Deleted" : "";
  const label = [
    SESSION_LINK_LABEL[link.kind],
    session && sessionAccountIdentity(accounts, session).short,
    note,
  ]
    .filter(Boolean)
    .join(" · ");
  const Icon = link.state === "deleted" ? LinkOffOutlined : LinkOutlined;
  const body = (
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
          color: open ? "primary.main" : "text.secondary",
          bgcolor: (theme) =>
            open
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
          color={open ? "text.primary" : "text.secondary"}
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
    </>
  );
  const frame = {
    display: "flex",
    alignItems: "center",
    gap: 1.25,
    width: "fit-content",
    maxWidth: "min(100%, 22rem)",
    minWidth: 0,
    minHeight: 44,
    boxSizing: "border-box",
    pl: 1,
    pr: link.state === "trash" ? 0.75 : 1.25,
    py: 0.75,
    border: 1,
    borderColor: "divider",
    borderRadius: 3,
    bgcolor: "background.paper",
  } as const;
  return open ? (
    <ButtonBase
      data-session-handoff-link={link.state}
      title="Open at the point it was handed off"
      onClick={(): void => openSessionLink(link, session)}
      sx={{ ...frame, "&:hover": { bgcolor: "action.hover" } }}
    >
      {body}
    </ButtonBase>
  ) : (
    <Box data-session-handoff-link={link.state} title={link.title} sx={frame}>
      {body}
    </Box>
  );
}
