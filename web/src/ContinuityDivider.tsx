import { Box, Button, Chip, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import OpenInNewRounded from "@mui/icons-material/OpenInNewRounded";
import SwapHorizRounded from "@mui/icons-material/SwapHorizRounded";
import VisibilityOutlined from "@mui/icons-material/VisibilityOutlined";
import { useState } from "react";
import { formatLineageActionLabel, type LineageHop } from "./ancestorHistory";
import { providerPresentation } from "./providerPresentation";
import type { SessionLinkRow } from "./sessionLinks";
import { openSessionLink, SessionHandoffSource, useRestoreSession } from "./SessionHandoffSource";
import { SessionHistoryPeek } from "./SessionHistoryPeek";
import { useStoreSelector } from "./store";
import { glyphSize } from "./surface/desktopSize";

function toLinkRow(hop: LineageHop): SessionLinkRow {
  return {
    id: hop.session_id,
    kind: hop.kind === "replace" ? "continued" : "forked",
    state: hop.parent_state === "live" ? "open" : hop.parent_state === "trashed" ? "trash" : "deleted",
    title: hop.title || hop.session_id,
    ...(hop.before_seq !== null ? { before: hop.before_seq } : {}),
  };
}

/** Calm visual seam between an inherited ancestor segment and the current
 *  branch. Shows provider transition, parent status, inline Restore for
 *  trashed sources, and one-click Open or read-only Peek. */

function DividerActions({
  hop,
  link,
  session,
  busy,
  onPeek,
  onRestore,
}: {
  hop: LineageHop;
  link: SessionLinkRow;
  session: ReturnType<typeof useStoreSelector<import("./protocol").SessionMeta | undefined>>;
  busy: boolean;
  onPeek: () => void;
  onRestore: () => void;
}): React.JSX.Element {
  const openable = hop.parent_state === "live";
  const peekable = hop.history_state === "live" || hop.history_state === "archived";
  return (
    <>
      {openable && (
        <Tooltip title="Open original session at handoff point">
          <IconButton
            size="small"
            aria-label="Open original session"
            onClick={(): void => openSessionLink(link, session)}
            sx={{ p: 0.35, color: "text.secondary" }}
          >
            <OpenInNewRounded sx={{ fontSize: glyphSize(14) }} />
          </IconButton>
        </Tooltip>
      )}
      {peekable && (
        <Tooltip title="Preview full snapshot in dialog">
          <IconButton
            size="small"
            data-session-handoff-peek
            aria-label="Preview original session"
            onClick={onPeek}
            sx={{ p: 0.35, color: "text.secondary" }}
          >
            <VisibilityOutlined sx={{ fontSize: glyphSize(14) }} />
          </IconButton>
        </Tooltip>
      )}
      {hop.parent_state === "trashed" && (
        <Button
          size="small"
          variant="outlined"
          disabled={busy}
          onClick={onRestore}
          sx={{ minWidth: 0, minHeight: 24, py: 0, px: 1, borderRadius: 999, fontSize: "0.7rem", textTransform: "none" }}
        >
          {busy ? "Restoring…" : "Restore"}
        </Button>
      )}
    </>
  );
}

export function ContinuityDivider({
  hop,
  childSessionId,
  childProvider,
}: {
  hop: LineageHop;
  childSessionId?: string | undefined;
  childProvider?: string | undefined;
}): React.JSX.Element {
  const session = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === hop.session_id),
  );
  const { busy, failure, restore } = useRestoreSession(hop.session_id);
  const [peekOpen, setPeekOpen] = useState(false);

  if (hop.history_state === "legacy_unknown") {
    return (
      <Box sx={{ my: 1.5, display: "flex", justifyContent: "center" }} data-continuity-divider="legacy">
        <SessionHandoffSource link={toLinkRow(hop)} />
      </Box>
    );
  }

  const link = toLinkRow(hop);
  const isReplace = hop.kind === "replace";
  const Icon = isReplace ? SwapHorizRounded : CallSplitOutlined;
  const sourceAgent = hop.provider ? providerPresentation(hop.provider).agent : "";
  const targetAgent = childProvider ? providerPresentation(childProvider).agent : "";
  const showTransition = Boolean(sourceAgent && targetAgent && sourceAgent !== targetAgent);
  const openable = hop.parent_state === "live";
  const peekable = hop.history_state === "live" || hop.history_state === "archived";

  return (
    <>
      <Box
        data-continuity-divider={hop.kind}
        data-session-handoff-link={link.state}
        sx={{
          my: 2,
          display: "flex",
          alignItems: "center",
          gap: 1.25,
          width: "100%",
          minWidth: 0,
        }}
      >
        <Box sx={{ flex: 1, height: "1px", bgcolor: "divider", minWidth: 12 }} />
        <Stack
          direction="row"
          spacing={0.75}
          alignItems="center"
          sx={(theme) => ({
            maxWidth: "100%",
            minWidth: 0,
            px: 1.25,
            py: 0.5,
            borderRadius: 999,
            border: 1,
            borderColor: "divider",
            bgcolor: alpha(theme.palette.background.paper, theme.palette.mode === "dark" ? 0.78 : 0.92),
          })}
        >
          <Icon sx={{ fontSize: glyphSize(15), color: "primary.main", flexShrink: 0 }} />
          <Typography variant="caption" color="text.secondary" noWrap sx={{ flexShrink: 0 }}>
            {formatLineageActionLabel(hop.kind)}
          </Typography>
          <Typography
            variant="caption"
            noWrap
            title={link.title}
            sx={{ fontWeight: 600, color: "text.primary", maxWidth: { xs: 140, sm: 240 } }}
          >
            {link.title}
          </Typography>
          {showTransition && (
            <Chip
              size="small"
              label={`${sourceAgent} → ${targetAgent}`}
              sx={{ height: 20, fontSize: "0.68rem", fontWeight: 500 }}
            />
          )}
          {hop.parent_state === "trashed" && (
            <Chip
              size="small"
              variant="outlined"
              label="In Trash"
              sx={{ height: 20, fontSize: "0.68rem", color: "text.secondary" }}
            />
          )}
          {hop.parent_state === "purged" && hop.history_state === "archived" && (
            <Chip
              size="small"
              variant="outlined"
              label="Archived"
              sx={{ height: 20, fontSize: "0.68rem", color: "text.secondary" }}
            />
          )}
          <DividerActions hop={hop} link={link} session={session} busy={busy} onPeek={(): void => setPeekOpen(true)} onRestore={restore} />
        </Stack>
        <Box sx={{ flex: 1, height: "1px", bgcolor: "divider", minWidth: 12 }} />
      </Box>
      {failure && (
        <Typography variant="caption" color="error" sx={{ display: "block", textAlign: "center", mb: 1 }}>
          {failure}
        </Typography>
      )}
      {peekable && peekOpen && (
        <SessionHistoryPeek
          open={peekOpen}
          onClose={(): void => setPeekOpen(false)}
          link={link}
          session={session}
          lineage={childSessionId ? { childSessionId, hopId: hop.session_id, edgeVersion: hop.edge_version, beforeSeq: hop.before_seq } : undefined}
          onOpenSession={openable ? (): void => openSessionLink(link, session) : undefined}
          onRestore={hop.parent_state === "trashed" ? restore : undefined}
          restoring={busy}
        />
      )}
    </>
  );
}
