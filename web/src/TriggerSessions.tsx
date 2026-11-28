// Pinned collapsible Triggers section above Trash in the Sessions sidebar.
// Keeps externally triggered runs isolated from ordinary human workspaces while
// exposing live status, placement, account, outcome, context usage, bulk
// cleanup, Session Info, Move to Trash, and one-click Take Over.

import {
  BoltOutlined,
  ChevronRight,
  DeleteOutline,
  DragIndicator,
  DriveFileMoveOutlined,
  ExpandMore,
  InfoOutlined,
  MoreVert,
} from "@mui/icons-material";
import { Box, Chip, IconButton, ListItemText, MenuItem, Stack, Typography } from "@mui/material";
import { alpha, type SxProps, type Theme } from "@mui/material/styles";
import { useCallback, useEffect, useMemo, useState } from "react";
import { AccountBadge } from "./AccountBadge";
import { sessionAccountIdentity } from "./accountIdentity";
import { documentNotice } from "./documents/DocumentNotifications";
import type { SessionMeta } from "./protocol";
import { ReliableListItemButton } from "./ReliableListItemButton";
import { SessionActionBadge } from "./SessionAttention";
import { SessionNextMessageBadge } from "./SessionContextCue";
import { InAppRoot } from "./SessionFolderUi";
import { SessionMachineBadge } from "./SessionMachineBadge";
import { SessionRowTitle } from "./SessionRowTitle";
import { ConfirmSheet } from "./Sheet";
import { sessionListProjectLabel } from "./sessionProject";
import { displayedSessionOrder } from "./sessionTree";
import { placeSessions, reorderSessions, trashItems } from "./store";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";
import { desktopSize } from "./surface/desktopSize";
import { formatTrashRemaining } from "./trashModel";
import {
  formatTriggerAge,
  mergeReorderedTriggers,
  summarizeTriggers,
  TRIGGERS_ROOT_KEY,
  type TriggerOutcomePresentation,
  triggerContextPercent,
  triggerDisplayMeta,
  triggerOutcomeMeta,
  triggerTimestampMs,
} from "./triggerModel";
import { type Sortable, useSortable } from "./useSortable";

const EXPANDED_KEY = "cowboy:sessions-triggers:expanded";
const SELECTED_ROW_FILL = 0.12;

function readExpanded(): boolean {
  try {
    return globalThis.localStorage.getItem(EXPANDED_KEY) === "true";
  } catch {
    return false;
  }
}

function outcomeChipSx(color: TriggerOutcomePresentation["color"]): SxProps<Theme> {
  return {
    height: "1.25rem",
    flexShrink: 0,
    fontSize: "0.6875rem",
    fontWeight: 500,
    border: "1px solid transparent",
    boxShadow: "none",
    "& .MuiChip-label": { px: "0.5rem" },
    color: (t: Theme) => {
      switch (color) {
        case "info":
          return t.palette.info.main;
        case "success":
          return t.palette.success.main;
        case "warning":
          return t.palette.warning.main;
        case "error":
          return t.palette.error.main;
        default:
          return t.palette.text.secondary;
      }
    },
    bgcolor: (t: Theme) => {
      const opacity = t.palette.mode === "dark" ? 0.22 : 0.14;
      switch (color) {
        case "info":
          return alpha(t.palette.info.main, opacity);
        case "success":
          return alpha(t.palette.success.main, opacity);
        case "warning":
          return alpha(t.palette.warning.main, opacity);
        case "error":
          return alpha(t.palette.error.main, opacity);
        default:
          return alpha(t.palette.text.secondary, opacity);
      }
    },
  };
}

export interface TriggerSessionsSectionProps {
  readonly sessions: readonly SessionMeta[];
  readonly accounts: SubscriptionSnapshot | undefined;
  readonly activeId: string | null;
  readonly onPick: (id: string) => void;
  readonly onRequestDelete: (session: SessionMeta) => void;
  readonly onRequestInfo: (session: SessionMeta) => void;
  readonly rowSx: (depth: number) => SxProps<Theme>;
  readonly now: number;
}

function TriggerRowSecondary({
  session,
  accounts,
  outcome,
  sourceChip,
  onRequestInfo,
  now,
}: {
  readonly session: SessionMeta;
  readonly accounts: SubscriptionSnapshot | undefined;
  readonly outcome: TriggerOutcomePresentation;
  readonly sourceChip: string | null;
  readonly onRequestInfo: (session: SessionMeta) => void;
  readonly now: number;
}): React.JSX.Element {
  const ctxPct = triggerContextPercent(session);
  const autoTrashLeft =
    session.trigger?.auto_trash_at_ms !== undefined
      ? Math.max(0, session.trigger.auto_trash_at_ms - now)
      : null;

  return (
    <Stack
      direction="row"
      alignItems="center"
      sx={{ minWidth: 0, maxWidth: "100%", flexWrap: "wrap", columnGap: 0.5, rowGap: 0.25 }}
    >
      <Chip size="small" label={outcome.label} sx={outcomeChipSx(outcome.color)} />
      <Typography variant="caption" color="text.secondary" noWrap sx={{ minWidth: 0 }}>
        {sessionListProjectLabel(session)}
      </Typography>
      <SessionMachineBadge session={session} onInfo={(): void => onRequestInfo(session)} compact />
      <AccountBadge account={sessionAccountIdentity(accounts, session)} />
      {sourceChip && (
        <Chip
          size="small"
          label={sourceChip}
          sx={{
            height: "1.25rem",
            flexShrink: 0,
            maxWidth: "10rem",
            fontSize: "0.6875rem",
            border: "1px solid transparent",
            boxShadow: "none",
            color: "text.secondary",
            bgcolor: (t) => alpha(t.palette.text.secondary, t.palette.mode === "dark" ? 0.16 : 0.1),
            "& .MuiChip-label": { px: "0.45rem", overflow: "hidden", textOverflow: "ellipsis" },
          }}
        />
      )}
      <SessionActionBadge session={session} />
      <SessionNextMessageBadge sessionId={session.id} />
      {ctxPct !== null && (
        <Typography
          variant="caption"
          color="text.disabled"
          sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}
          title="Context window usage"
        >
          {ctxPct}% ctx
        </Typography>
      )}
      {autoTrashLeft !== null && (
        <Typography
          variant="caption"
          color="text.disabled"
          sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}
          title="Moves to Trash automatically when this timer expires"
        >
          · Trash {formatTrashRemaining(autoTrashLeft)}
        </Typography>
      )}
    </Stack>
  );
}

function TriggerRow({
  session,
  accounts,
  selected,
  sortable,
  onPick,
  onOpenMenu,
  onRequestInfo,
  rowSx,
  now,
}: {
  readonly session: SessionMeta;
  readonly accounts: SubscriptionSnapshot | undefined;
  readonly selected: boolean;
  readonly sortable: Sortable;
  readonly onPick: (id: string) => void;
  readonly onOpenMenu: (session: SessionMeta) => void;
  readonly onRequestInfo: (session: SessionMeta) => void;
  readonly rowSx: (depth: number) => SxProps<Theme>;
  readonly now: number;
}): React.JSX.Element {
  const outcome = triggerOutcomeMeta(session);
  const display = triggerDisplayMeta(session);
  const displaySession = useMemo(
    () => (display.title === session.title ? session : { ...session, title: display.title }),
    [display.title, session],
  );
  const ts = triggerTimestampMs(session);
  const ageLabel = ts !== null ? formatTriggerAge(now - ts) : null;
  const sourceChip = display.source && display.source !== display.title ? display.source : null;
  const dragging = sortable.draggingId === session.id;

  return (
    <ReliableListItemButton
      selected={selected}
      data-desktop-item={session.id}
      data-trigger-session-row={session.id}
      ref={sortable.registerItem(session.id)}
      style={sortable.itemStyle(session.id)}
      onActivate={(): void => onPick(session.id)}
      sx={[
        rowSx(1) as object,
        selected
          ? {
              "&&.Mui-selected, &&[data-desktop-current='true']": {
                bgcolor: (t: Theme) => alpha(t.palette.primary.main, SELECTED_ROW_FILL),
                boxShadow: "none",
              },
              "&&.Mui-selected:hover, &&[data-desktop-current='true']:hover": {
                bgcolor: (t: Theme) => alpha(t.palette.primary.main, SELECTED_ROW_FILL + 0.04),
              },
            }
          : {},
        dragging
          ? {
              bgcolor: "background.paper",
              boxShadow: (t: Theme) =>
                `0 8px 24px ${alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.5 : 0.18)}`,
              "&::before": { display: "none" },
            }
          : {},
      ]}
    >
      <Box
        className="cowboy-session-grip-slot"
        sx={{
          position: "relative",
          width: desktopSize(44),
          height: desktopSize(44),
          flexShrink: 0,
          display: "grid",
          placeItems: "center",
        }}
      >
        <IconButton
          className="cowboy-session-grip"
          {...sortable.handleProps(session.id)}
          aria-label="Drag to reorder"
          sx={{ width: desktopSize(44), height: desktopSize(44), color: "text.disabled" }}
        >
          <DragIndicator sx={{ fontSize: "1.25rem" }} />
        </IconButton>
      </Box>
      <ListItemText
        sx={{ my: 0.25, minWidth: 0 }}
        primary={
          <Stack direction="row" spacing={0.5} alignItems="center" sx={{ minWidth: 0 }}>
            <Box sx={{ minWidth: 0, flex: 1 }}>
              <SessionRowTitle session={displaySession} active={selected} />
            </Box>
            {ageLabel && (
              <Typography
                variant="caption"
                color="text.disabled"
                sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}
              >
                {ageLabel}
              </Typography>
            )}
          </Stack>
        }
        secondary={
          <TriggerRowSecondary
            session={session}
            accounts={accounts}
            outcome={outcome}
            sourceChip={sourceChip}
            onRequestInfo={onRequestInfo}
            now={now}
          />
        }
        slotProps={{
          primary: { component: "div" },
          secondary: { component: "div", noWrap: true, variant: "caption" },
        }}
      />
      <IconButton
        className="cowboy-session-actions"
        aria-label={`Trigger actions for ${display.title}`}
        onClick={(event): void => {
          event.stopPropagation();
          onOpenMenu(session);
        }}
        sx={{ ml: 0.5, width: desktopSize(44), height: desktopSize(44), flexShrink: 0 }}
      >
        <MoreVert sx={{ fontSize: "1.5rem" }} />
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
  onInspectRow,
  onTakeOver,
  onTrashRow,
  now,
}: {
  readonly menuOpen: boolean;
  readonly onCloseMenu: () => void;
  readonly settledCount: number;
  readonly cleaning: boolean;
  readonly onTrashSettled: () => void;
  readonly selectedRow: SessionMeta | null;
  readonly onCloseRow: () => void;
  readonly onInspectRow: (session: SessionMeta) => void;
  readonly onTakeOver: (session: SessionMeta) => void;
  readonly onTrashRow: (session: SessionMeta) => void;
  readonly now: number;
}): React.JSX.Element {
  const rowMeta = selectedRow ? triggerDisplayMeta(selectedRow) : null;
  const rowOutcome = selectedRow ? triggerOutcomeMeta(selectedRow) : null;
  const autoTrashLeft =
    selectedRow?.trigger?.auto_trash_at_ms !== undefined
      ? Math.max(0, selectedRow.trigger.auto_trash_at_ms - now)
      : null;

  return (
    <InAppRoot>
      <ConfirmSheet open={menuOpen} onClose={onCloseMenu} title="Triggers">
        <MenuItem
          disabled={settledCount === 0 || cleaning}
          onClick={(): void => {
            onTrashSettled();
          }}
        >
          <DeleteOutline fontSize="small" sx={{ mr: 1.25, color: "text.secondary" }} />
          <ListItemText
            primary={`Move settled runs to Trash (${String(settledCount)})`}
            secondary={
              settledCount > 0
                ? "Running and escalated runs stay active"
                : "No settled runs to clean up"
            }
          />
        </MenuItem>
      </ConfirmSheet>

      <ConfirmSheet
        open={selectedRow !== null}
        onClose={onCloseRow}
        title={rowMeta?.title ?? "Trigger run"}
      >
        {selectedRow && rowMeta && rowOutcome && (
          <>
            <Stack
              direction="row"
              alignItems="center"
              sx={{ px: 2, pb: 1, flexWrap: "wrap", columnGap: 0.75, rowGap: 0.5 }}
            >
              <Chip size="small" label={rowOutcome.label} sx={outcomeChipSx(rowOutcome.color)} />
              {rowMeta.source && (
                <Typography variant="caption" color="text.secondary">
                  source: {rowMeta.source}
                </Typography>
              )}
              {rowMeta.correlationKey && (
                <Typography
                  variant="caption"
                  color="text.secondary"
                  sx={{ fontFamily: "monospace" }}
                >
                  key: {rowMeta.correlationKey}
                </Typography>
              )}
              {autoTrashLeft !== null && (
                <Typography variant="caption" color="text.disabled">
                  auto-trash in {formatTrashRemaining(autoTrashLeft)}
                </Typography>
              )}
            </Stack>
            <MenuItem onClick={(): void => onInspectRow(selectedRow)}>
              <InfoOutlined fontSize="small" sx={{ mr: 1.25, color: "text.secondary" }} />
              <ListItemText
                primary="Info"
                secondary="Placement, account, context usage, and logs"
              />
            </MenuItem>
            <MenuItem onClick={(): void => onTakeOver(selectedRow)}>
              <DriveFileMoveOutlined fontSize="small" sx={{ mr: 1.25, color: "text.secondary" }} />
              <ListItemText
                primary="Take over to workspace"
                secondary="Promote to a regular session and disable auto-trash"
              />
            </MenuItem>
            <MenuItem onClick={(): void => onTrashRow(selectedRow)}>
              <DeleteOutline fontSize="small" sx={{ mr: 1.25, color: "error.main" }} />
              <ListItemText
                primary="Move to Trash"
                secondary="Hibernate worker and keep transcript in Trash"
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
  live,
  onToggle,
  onOpenMenu,
  rowSx,
}: {
  readonly open: boolean;
  readonly count: number;
  readonly working: number;
  readonly attention: number;
  readonly live: number;
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
      {!open && (working > 0 || attention > 0 || live > 0) && (
        <Stack direction="row" spacing={0.5} alignItems="center" sx={{ flexShrink: 0, mr: 0.5 }}>
          {working > 0 && (
            <Chip size="small" label={`${String(working)} live`} sx={outcomeChipSx("info")} />
          )}
          {attention > 0 && (
            <Chip size="small" label={`${String(attention)} attn`} sx={outcomeChipSx("warning")} />
          )}
          {working === 0 && attention === 0 && live > 0 && (
            <Chip size="small" label={`${String(live)} awake`} sx={outcomeChipSx("success")} />
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
  accounts,
  activeId,
  onPick,
  onRequestDelete,
  onRequestInfo,
  rowSx,
  now,
}: TriggerSessionsSectionProps): React.JSX.Element | null {
  const [open, setOpen] = useState<boolean>(readExpanded);
  const [menuOpen, setMenuOpen] = useState(false);
  const [selectedRow, setSelectedRow] = useState<SessionMeta | null>(null);
  const [cleaning, setCleaning] = useState(false);
  const summary = useMemo(() => summarizeTriggers(sessions), [sessions]);
  const triggerById = useMemo(
    () => new Map(summary.sessions.map((session) => [session.id, session])),
    [summary.sessions],
  );
  const triggerIds = useMemo(() => summary.sessions.map((session) => session.id), [summary.sessions]);
  const onReorderTriggers = useCallback(
    (nextIds: string[]): void => {
      const merged = mergeReorderedTriggers(sessions, nextIds);
      reorderSessions(displayedSessionOrder(merged));
    },
    [sessions],
  );
  // Scoped strictly to rows inside Triggers: rows reorder within Triggers only
  // and cannot be dragged out into Global/folders, nor can the Triggers header
  // itself be dragged.
  const sortable = useSortable({
    ids: triggerIds,
    onReorder: onReorderTriggers,
    optimisticReorder: true,
  });

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

  const inspectRow = useCallback(
    (session: SessionMeta): void => {
      setSelectedRow(null);
      onRequestInfo(session);
    },
    [onRequestInfo],
  );

  const takeOver = useCallback((session: SessionMeta): void => {
    placeSessions([session.id], null);
    setSelectedRow(null);
    documentNotice(`Promoted “${triggerDisplayMeta(session).title}” to your workspace.`);
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
        live={summary.activity.live}
        onToggle={toggle}
        onOpenMenu={(): void => setMenuOpen(true)}
        rowSx={rowSx}
      />

      {open &&
        sortable.order.map((id) => {
          const session = triggerById.get(id);
          if (!session) return null;
          return (
            <TriggerRow
              key={session.id}
              session={session}
              accounts={accounts}
              selected={activeId === session.id}
              sortable={sortable}
              onPick={onPick}
              onOpenMenu={setSelectedRow}
              onRequestInfo={onRequestInfo}
              rowSx={rowSx}
              now={now}
            />
          );
        })}

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
        onInspectRow={inspectRow}
        onTakeOver={takeOver}
        onTrashRow={trashRow}
        now={now}
      />
    </>
  );
}
