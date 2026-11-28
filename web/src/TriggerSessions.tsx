// Pinned collapsible Triggers section above Trash in the Sessions sidebar.
// Keeps externally triggered runs isolated from ordinary human workspaces while
// exposing live status, outcome badges, bulk cleanup, Move to Trash, and
// one-click Take Over (promotion to ordinary workspace session).

import {
  BoltOutlined,
  ChevronRight,
  DeleteOutline,
  DriveFileMoveOutlined,
  ExpandMore,
  MoreVert,
} from "@mui/icons-material";
import { Box, Chip, IconButton, ListItemText, MenuItem, Stack, Typography } from "@mui/material";
import type { SxProps, Theme } from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { SessionMeta } from "./protocol";
import { ReliableListItemButton } from "./ReliableListItemButton";
import { InAppRoot } from "./SessionFolderUi";
import { ConfirmSheet } from "./Sheet";
import { documentNotice } from "./documents/DocumentNotifications";
import { placeSessions, trashItems } from "./store";
import { desktopSize } from "./surface/desktopSize";
import { formatTrashRemaining } from "./trashModel";
import { summarizeTriggers, TRIGGERS_ROOT_KEY, triggerOutcomeMeta } from "./triggerModel";

const EXPANDED_KEY = "cowboy:sessions-triggers:expanded";

function readExpanded(): boolean {
  try {
    return globalThis.localStorage.getItem(EXPANDED_KEY) === "true";
  } catch {
    return false;
  }
}

export interface TriggerSessionsSectionProps {
  readonly sessions: readonly SessionMeta[];
  readonly activeId: string | null;
  readonly onPick: (id: string) => void;
  readonly onRequestDelete: (session: SessionMeta) => void;
  readonly rowSx: (depth: number) => SxProps<Theme>;
  readonly now: number;
}

function TriggerRow({
  session,
  selected,
  onPick,
  onOpenMenu,
  rowSx,
  now,
}: {
  readonly session: SessionMeta;
  readonly selected: boolean;
  readonly onPick: (id: string) => void;
  readonly onOpenMenu: (session: SessionMeta) => void;
  readonly rowSx: (depth: number) => SxProps<Theme>;
  readonly now: number;
}): React.JSX.Element {
  const outcome = triggerOutcomeMeta(session);
  const autoTrashLeft =
    session.trigger?.auto_trash_at_ms !== undefined
      ? Math.max(0, session.trigger.auto_trash_at_ms - now)
      : null;
  return (
    <ReliableListItemButton
      selected={selected}
      data-desktop-item={session.id}
      data-trigger-session-row={session.id}
      onActivate={(): void => onPick(session.id)}
      sx={[rowSx(1) as object]}
    >
      <ListItemText
        primary={
          <Typography variant="body2" noWrap sx={{ fontWeight: selected ? 600 : 400 }}>
            {session.title || session.id}
          </Typography>
        }
        secondary={
          <Stack direction="row" spacing={0.5} alignItems="center" sx={{ mt: 0.25, minWidth: 0 }}>
            <Chip
              size="small"
              color={outcome.color}
              label={outcome.label}
              sx={{
                height: "1.2rem",
                fontSize: "0.6875rem",
                "& .MuiChip-label": { px: "0.45rem" },
              }}
            />
            {session.workspace_name && (
              <Typography variant="caption" color="text.secondary" noWrap>
                {session.workspace_name}
              </Typography>
            )}
            {autoTrashLeft !== null && (
              <Typography
                variant="caption"
                color="text.disabled"
                sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}
                title="Moves to Trash automatically when this timer expires"
              >
                · {formatTrashRemaining(autoTrashLeft)}
              </Typography>
            )}
          </Stack>
        }
        slotProps={{ primary: { component: "div" }, secondary: { component: "div" } }}
      />
      <IconButton
        className="cowboy-session-actions"
        aria-label={`Trigger actions for ${session.title || session.id}`}
        onClick={(event): void => {
          event.stopPropagation();
          onOpenMenu(session);
        }}
        sx={{ ml: 0.5, width: desktopSize(44), height: desktopSize(44), flexShrink: 0 }}
      >
        <MoreVert sx={{ fontSize: "1.35rem" }} />
      </IconButton>
    </ReliableListItemButton>
  );
}

function TriggerSheets({
  menuOpen,
  onCloseMenu,
  settledCount,
  cleaning,
  onTrashSettled,
  selectedRow,
  onCloseRow,
  onTakeOver,
  onTrashRow,
}: {
  readonly menuOpen: boolean;
  readonly onCloseMenu: () => void;
  readonly settledCount: number;
  readonly cleaning: boolean;
  readonly onTrashSettled: () => void;
  readonly selectedRow: SessionMeta | null;
  readonly onCloseRow: () => void;
  readonly onTakeOver: (session: SessionMeta) => void;
  readonly onTrashRow: (session: SessionMeta) => void;
}): React.JSX.Element {
  return (
    <InAppRoot>
      <ConfirmSheet open={menuOpen} onClose={onCloseMenu} title="Triggers">
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
          Externally triggered runs stay isolated here, auto-hibernate after settling, and move to
          Trash automatically unless promoted to your workspace.
        </Typography>
        <MenuItem disabled={settledCount === 0 || cleaning} onClick={onTrashSettled}>
          <DeleteOutline fontSize="small" sx={{ mr: 1.25, color: "text.secondary" }} />
          <ListItemText
            primary={`Move settled runs to Trash (${String(settledCount)})`}
            secondary="Leaves active and escalated runs untouched"
          />
        </MenuItem>
      </ConfirmSheet>
      <ConfirmSheet
        open={selectedRow !== null}
        onClose={onCloseRow}
        title={selectedRow?.title || "Trigger run"}
      >
        {selectedRow && (
          <>
            <MenuItem onClick={(): void => onTakeOver(selectedRow)}>
              <DriveFileMoveOutlined fontSize="small" sx={{ mr: 1.25, color: "text.secondary" }} />
              <ListItemText
                primary="Take over to workspace"
                secondary="Move out of Triggers and disable auto-cleanup"
              />
            </MenuItem>
            <MenuItem onClick={(): void => onTrashRow(selectedRow)}>
              <DeleteOutline fontSize="small" sx={{ mr: 1.25, color: "error.main" }} />
              <ListItemText
                primary="Move to Trash"
                slotProps={{ primary: { color: "error.main" } }}
              />
            </MenuItem>
          </>
        )}
      </ConfirmSheet>
    </InAppRoot>
  );
}

function TriggerHeader({
  open,
  count,
  working,
  attention,
  onToggle,
  onOpenMenu,
  rowSx,
}: {
  readonly open: boolean;
  readonly count: number;
  readonly working: number;
  readonly attention: number;
  readonly onToggle: () => void;
  readonly onOpenMenu: () => void;
  readonly rowSx: (depth: number) => SxProps<Theme>;
}): React.JSX.Element {
  return (
    <ReliableListItemButton
      data-desktop-item={TRIGGERS_ROOT_KEY}
      data-session-triggers-root
      aria-expanded={open}
      onActivate={onToggle}
      sx={[rowSx(0) as object, { mt: 0.5, borderTop: 1, borderColor: "divider" }]}
    >
      <Box
        className="cowboy-session-grip cowboy-folder-chevron"
        aria-hidden
        sx={{
          width: desktopSize(44),
          height: desktopSize(44),
          flexShrink: 0,
          display: "grid",
          placeItems: "center",
          color: "text.secondary",
        }}
      >
        {open ? (
          <ExpandMore sx={{ fontSize: "1.5rem" }} />
        ) : (
          <ChevronRight sx={{ fontSize: "1.5rem" }} />
        )}
      </Box>
      <ListItemText
        primary={
          <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
            <BoltOutlined fontSize="medium" sx={{ flexShrink: 0, color: "text.secondary" }} />
            <Typography variant="body2" noWrap sx={{ minWidth: 0, fontWeight: 600 }}>
              Triggers
            </Typography>
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}
            >
              {count}
            </Typography>
          </Stack>
        }
        slotProps={{ primary: { component: "div" } }}
      />
      {!open && (working > 0 || attention > 0) && (
        <Stack direction="row" spacing={0.5} alignItems="center" sx={{ flexShrink: 0, mr: 0.5 }}>
          {working > 0 && (
            <Chip
              size="small"
              color="info"
              label={`${String(working)} live`}
              sx={{
                height: "1.25rem",
                fontSize: "0.6875rem",
                "& .MuiChip-label": { px: "0.45rem" },
              }}
            />
          )}
          {attention > 0 && (
            <Chip
              size="small"
              color="warning"
              label={`${String(attention)} attn`}
              sx={{
                height: "1.25rem",
                fontSize: "0.6875rem",
                "& .MuiChip-label": { px: "0.45rem" },
              }}
            />
          )}
        </Stack>
      )}
      <IconButton
        className="cowboy-session-actions"
        aria-label="Triggers actions"
        onClick={(event): void => {
          event.stopPropagation();
          onOpenMenu();
        }}
        sx={{ ml: 0.5, width: desktopSize(44), height: desktopSize(44), flexShrink: 0 }}
      >
        <MoreVert sx={{ fontSize: "1.5rem" }} />
      </IconButton>
    </ReliableListItemButton>
  );
}

export function TriggerSessionsSection({
  sessions,
  activeId,
  onPick,
  onRequestDelete,
  rowSx,
  now,
}: TriggerSessionsSectionProps): React.JSX.Element | null {
  const [open, setOpen] = useState<boolean>(readExpanded);
  const [menuOpen, setMenuOpen] = useState(false);
  const [selectedRow, setSelectedRow] = useState<SessionMeta | null>(null);
  const [cleaning, setCleaning] = useState(false);
  const summary = useMemo(() => summarizeTriggers(sessions), [sessions]);

  useEffect(() => {
    try {
      globalThis.localStorage.setItem(EXPANDED_KEY, open ? "true" : "false");
    } catch {
      // Ignore storage denial.
    }
  }, [open]);

  const toggle = useCallback((): void => setOpen((prev) => !prev), []);

  const trashSettled = useCallback(async (): Promise<void> => {
    if (summary.settledIds.length === 0 || cleaning) return;
    setCleaning(true);
    try {
      await trashItems({ sessions: [...summary.settledIds], folders: [] });
      documentNotice(`Moved ${String(summary.settledIds.length)} settled trigger run(s) to Trash.`);
      setMenuOpen(false);
    } catch (error) {
      documentNotice(
        error instanceof Error ? error.message : "Could not move settled runs to Trash.",
      );
    } finally {
      setCleaning(false);
    }
  }, [cleaning, summary.settledIds]);

  const takeOver = useCallback((session: SessionMeta): void => {
    placeSessions([session.id], null);
    setSelectedRow(null);
    documentNotice(`Promoted “${session.title || "Session"}” to your workspace.`);
  }, []);

  const trashRow = useCallback(
    (session: SessionMeta): void => {
      setSelectedRow(null);
      onRequestDelete(session);
    },
    [onRequestDelete],
  );

  if (summary.sessions.length === 0) return null;

  return (
    <>
      <TriggerHeader
        open={open}
        count={summary.sessions.length}
        working={summary.activity.working}
        attention={summary.activity.attention}
        onToggle={toggle}
        onOpenMenu={(): void => setMenuOpen(true)}
        rowSx={rowSx}
      />

      {open &&
        summary.sessions.map((session) => (
          <TriggerRow
            key={session.id}
            session={session}
            selected={activeId === session.id}
            onPick={onPick}
            onOpenMenu={setSelectedRow}
            rowSx={rowSx}
            now={now}
          />
        ))}

      <TriggerSheets
        menuOpen={menuOpen}
        onCloseMenu={(): void => setMenuOpen(false)}
        settledCount={summary.settledIds.length}
        cleaning={cleaning}
        onTrashSettled={(): void => {
          void trashSettled();
        }}
        selectedRow={selectedRow}
        onCloseRow={(): void => setSelectedRow(null)}
        onTakeOver={takeOver}
        onTrashRow={trashRow}
      />
    </>
  );
}
