import { Box, Button, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import { alpha, type Theme } from "@mui/material/styles";
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

function tonalBadgeSx(theme: Theme, muted = false): Record<string, unknown> {
  const dark = theme.palette.mode === "dark";
  return {
    display: "inline-flex",
    alignItems: "center",
    height: 22,
    px: 0.9,
    borderRadius: 999,
    fontSize: "0.7rem",
    fontWeight: muted ? 500 : 550,
    lineHeight: 1,
    whiteSpace: "nowrap",
    flexShrink: 0,
    color: "text.secondary",
    bgcolor: alpha(theme.palette.text.primary, dark ? (muted ? 0.065 : 0.09) : (muted ? 0.05 : 0.07)),
  };
}

function DividerActions({
  hop,
  link,
  session,
  busy,
  compact,
  onPeek,
  onRestore,
}: {
  hop: LineageHop;
  link: SessionLinkRow;
  session: ReturnType<typeof useStoreSelector<import("./protocol").SessionMeta | undefined>>;
  busy: boolean;
  compact: boolean;
  onPeek: () => void;
  onRestore: () => void;
}): React.JSX.Element {
  const openable = hop.parent_state === "live";
  const peekable = hop.history_state === "live" || hop.history_state === "archived";
  const iconButtonSx = (theme: Theme) => ({
    width: compact ? 26 : 32,
    height: compact ? 26 : 32,
    minWidth: compact ? 26 : 32,
    minHeight: compact ? 26 : 32,
    p: 0,
    borderRadius: 999,
    color: "text.secondary",
    bgcolor: compact
      ? "transparent"
      : alpha(theme.palette.text.primary, theme.palette.mode === "dark" ? 0.075 : 0.055),
    "&:hover": {
      color: "text.primary",
      bgcolor: alpha(theme.palette.text.primary, theme.palette.mode === "dark" ? 0.13 : 0.09),
    },
  });

  return (
    <Stack direction="row" spacing={compact ? 0.4 : 0.75} alignItems="center" sx={{ flexShrink: 0 }}>
      {openable && (
        <Tooltip title="Open original session at handoff point">
          <IconButton
            size="small"
            aria-label="Open original session"
            onClick={(): void => openSessionLink(link, session)}
            sx={iconButtonSx}
          >
            <OpenInNewRounded sx={{ fontSize: glyphSize(compact ? 14 : 16) }} />
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
            sx={iconButtonSx}
          >
            <VisibilityOutlined sx={{ fontSize: glyphSize(compact ? 14 : 16) }} />
          </IconButton>
        </Tooltip>
      )}
      {hop.parent_state === "trashed" && (
        <Button
          size="small"
          variant="text"
          disabled={busy}
          onClick={onRestore}
          sx={(theme) => ({
            minWidth: "auto",
            minHeight: compact ? 24 : 30,
            height: compact ? 24 : 30,
            py: 0,
            px: compact ? 1.15 : 1.4,
            borderRadius: 999,
            fontSize: compact ? "0.7rem" : "0.75rem",
            fontWeight: 600,
            lineHeight: 1,
            whiteSpace: "nowrap",
            textTransform: "none",
            color: "primary.main",
            bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.16 : 0.10),
            border: `1px solid ${alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.34 : 0.24)}`,
            "&:hover": {
              bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.24 : 0.16),
            },
          })}
        >
          {busy ? "Restoring…" : "Restore"}
        </Button>
      )}
    </Stack>
  );
}

function MobileContinuityCard({
  hop,
  link,
  session,
  busy,
  transitionLabel,
  statusLabel,
  hasSecondaryRow,
  onPeek,
  onRestore,
}: {
  hop: LineageHop;
  link: SessionLinkRow;
  session: ReturnType<typeof useStoreSelector<import("./protocol").SessionMeta | undefined>>;
  busy: boolean;
  transitionLabel: string | null;
  statusLabel: string | null;
  hasSecondaryRow: boolean;
  onPeek: () => void;
  onRestore: () => void;
}): React.JSX.Element {
  const Icon = hop.kind === "replace" ? SwapHorizRounded : CallSplitOutlined;
  return (
    <Box
      data-continuity-card="mobile"
      sx={(theme) => ({
        display: { xs: "flex", sm: "none" },
        flexDirection: "column",
        width: "100%",
        minWidth: 0,
        px: 1.5,
        py: 1.1,
        borderRadius: "14px",
        border: `1px solid ${alpha(theme.palette.divider, theme.palette.mode === "dark" ? 0.85 : 0.72)}`,
        bgcolor: alpha(theme.palette.background.paper, theme.palette.mode === "dark" ? 0.68 : 0.9),
      })}
    >
      <Stack direction="row" spacing={0.85} alignItems="center" sx={{ minWidth: 0 }}>
        <Box
          aria-hidden
          sx={(theme) => ({
            width: 24,
            height: 24,
            borderRadius: "8px",
            flexShrink: 0,
            display: "grid",
            placeItems: "center",
            color: "primary.main",
            bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.15 : 0.1),
          })}
        >
          <Icon sx={{ fontSize: glyphSize(14) }} />
        </Box>
        <Stack direction="row" spacing={0.5} alignItems="baseline" sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="caption" color="text.secondary" noWrap sx={{ flexShrink: 0 }}>
            {formatLineageActionLabel(hop.kind)}
          </Typography>
          <Typography variant="caption" noWrap title={link.title} sx={{ fontWeight: 650, color: "text.primary", minWidth: 0, flex: 1 }}>
            {link.title}
          </Typography>
        </Stack>
        {statusLabel && <Box component="span" sx={(t) => tonalBadgeSx(t, true)}>{statusLabel}</Box>}
      </Stack>
      {hasSecondaryRow && (
        <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1} sx={{ mt: 0.85, minWidth: 0 }}>
          {transitionLabel ? <Box component="span" sx={(t) => tonalBadgeSx(t, false)}>{transitionLabel}</Box> : <Box />}
          <DividerActions hop={hop} link={link} session={session} busy={busy} compact={false} onPeek={onPeek} onRestore={onRestore} />
        </Stack>
      )}
    </Box>
  );
}

/** Calm visual seam between an inherited ancestor segment and the current
 *  branch. Renders a structured two-row card on phone widths (so session
 *  titles and Restore pills never crush into narrow ovals) and a centered
 *  single-line ribbon on wider screens. */
export function ContinuityDivider({
  hop,
  childSessionId,
  childProvider,
}: {
  hop: LineageHop;
  childSessionId?: string | undefined;
  childProvider?: string | undefined;
}): React.JSX.Element {
  const session = useStoreSelector((state) => state.sessions.find((entry) => entry.id === hop.session_id));
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
  const Icon = hop.kind === "replace" ? SwapHorizRounded : CallSplitOutlined;
  const sourceAgent = hop.provider ? providerPresentation(hop.provider).agent : "";
  const targetAgent = childProvider ? providerPresentation(childProvider).agent : "";
  const transitionLabel = sourceAgent && targetAgent && sourceAgent !== targetAgent
    ? `${sourceAgent} → ${targetAgent}`
    : null;
  const openable = hop.parent_state === "live";
  const peekable = hop.history_state === "live" || hop.history_state === "archived";
  const statusLabel = hop.parent_state === "trashed"
    ? "In Trash"
    : hop.parent_state === "purged" && hop.history_state === "archived"
    ? "Archived"
    : null;
  const hasSecondaryRow = Boolean(transitionLabel) || openable || peekable || hop.parent_state === "trashed";

  return (
    <>
      <Box
        data-continuity-divider={hop.kind}
        data-session-handoff-link={link.state}
        sx={{ my: 1.75, display: "flex", alignItems: "center", gap: { xs: 0, sm: 1.25 }, width: "100%", minWidth: 0 }}
      >
        <Box sx={{ display: { xs: "none", sm: "block" }, flex: 1, height: "1px", bgcolor: "divider", minWidth: 12 }} />
        <MobileContinuityCard
          hop={hop}
          link={link}
          session={session}
          busy={busy}
          transitionLabel={transitionLabel}
          statusLabel={statusLabel}
          hasSecondaryRow={hasSecondaryRow}
          onPeek={(): void => setPeekOpen(true)}
          onRestore={restore}
        />
        <Stack
          direction="row"
          spacing={0.75}
          alignItems="center"
          data-continuity-card="desktop"
          sx={(theme) => ({
            display: { xs: "none", sm: "inline-flex" },
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
          <Typography variant="caption" noWrap title={link.title} sx={{ fontWeight: 600, color: "text.primary", maxWidth: 240 }}>
            {link.title}
          </Typography>
          {transitionLabel && <Box component="span" sx={(t) => tonalBadgeSx(t, false)}>{transitionLabel}</Box>}
          {statusLabel && <Box component="span" sx={(t) => tonalBadgeSx(t, true)}>{statusLabel}</Box>}
          <DividerActions hop={hop} link={link} session={session} busy={busy} compact onPeek={(): void => setPeekOpen(true)} onRestore={restore} />
        </Stack>
        <Box sx={{ display: { xs: "none", sm: "block" }, flex: 1, height: "1px", bgcolor: "divider", minWidth: 12 }} />
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
