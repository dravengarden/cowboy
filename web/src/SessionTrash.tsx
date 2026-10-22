// Sessions Trash chrome (docs/sessions-folders.md#trash), shared by the Mobile
// drawer and the Desktop rail. The Trash is one container pinned after the
// whole Sessions tree. It starts collapsed and only a direct toggle opens it:
// reveal, Focus and Expand all never do. Inside, deleted folders keep their
// structure and every entry shows how long it has left.

import {
  Box,
  Button,
  IconButton,
  ListItemText,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import type { SxProps, Theme } from "@mui/material";
import {
  ChevronRight,
  DeleteForeverOutlined,
  DeleteOutline,
  DescriptionOutlined,
  ExpandMore,
  FolderOutlined,
  MoreVert,
  RestoreFromTrashOutlined,
  ChatBubbleOutline,
} from "@mui/icons-material";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { DraftMetadata } from "./documents/model";
import { draftRepository } from "./documents/store";
import { documentNotice } from "./documents/DocumentNotifications";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { ReliableListItemButton } from "./ReliableListItemButton";
import { InAppRoot } from "./SessionFolderUi";
import { ConfirmSheet } from "./Sheet";
import { sessionListProjectLabel } from "./sessionProject";
import { productSessionSignal } from "./productSessionEnd";
import { purgeItems, restoreItems, useStoreSelector } from "./store";
import { desktopSize } from "./surface/desktopSize";
import {
  buildTrashTree,
  DEFAULT_TRASH_RETENTION_DAYS,
  describeTrashAge,
  describeTrashRemaining,
  formatTrashRemaining,
  TRASH_RETENTION_CHOICES,
  TRASH_ROOT_KEY,
  TRASH_URGENT_MS,
  trashDeadline,
  type TrashRow,
  type TrashTree,
} from "./trashModel";
import { sessionFolderById, sessionFolderLocation, type SessionFoldersValue } from "./sessionFolders";

const EXPANDED_KEY = "cowboy:sessions-trash:expanded";

function readExpanded(): ReadonlySet<string> {
  try {
    const parsed: unknown = JSON.parse(globalThis.localStorage.getItem(EXPANDED_KEY) ?? "[]");
    return new Set(Array.isArray(parsed) ? parsed.filter((key): key is string => typeof key === "string") : []);
  } catch {
    return new Set();
  }
}

/** Open Trash rows on this device. Empty by default: Trash starts closed. */
export function useTrashExpanded(): readonly [ReadonlySet<string>, (key: string, open?: boolean) => void] {
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(readExpanded);
  useEffect(() => {
    try {
      globalThis.localStorage.setItem(EXPANDED_KEY, JSON.stringify([...expanded]));
    } catch {
      // Storage may be refused; the fold then lasts this page.
    }
  }, [expanded]);
  const toggle = useCallback((key: string, open?: boolean): void => {
    setExpanded((previous) => {
      const next = new Set(previous);
      if (open ?? !next.has(key)) next.add(key);
      else next.delete(key);
      return next;
    });
  }, []);
  return [expanded, toggle] as const;
}

/** A clock that ticks once a minute while `active`: countdowns read in
 *  minutes at their finest. */
export function useMinuteClock(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return undefined;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, [active]);
  return now;
}

interface TrashPreferences {
  readonly preferences: { readonly revision: number; readonly values: { readonly retention_days?: number } };
  readonly dataset: { readonly dataset_id: string };
}

/** The owner's Trash retention (`preferences.trash`), stored on the Controller
 *  so every device and the Cowboy MCP agree. */
export function useTrashRetention(connected: boolean): {
  readonly days: number;
  readonly saving: boolean;
  readonly save: (days: number) => Promise<void>;
} {
  const [data, setData] = useState<TrashPreferences>();
  const [saving, setSaving] = useState(false);
  const load = useCallback(async (): Promise<void> => {
    try {
      const response = await fetch("/api/preferences/trash", { cache: "no-store", signal: productSessionSignal() });
      if (!response.ok) return;
      setData(await response.json() as TrashPreferences);
    } catch {
      // Offline: the default deadline is shown until the Controller answers.
    }
  }, []);
  useEffect(() => {
    if (!connected) return undefined;
    void load();
    const refresh = (event: Event): void => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (scopes && scopes.trash === undefined) return;
      void load();
    };
    globalThis.addEventListener("cowboy:preferences-changed", refresh);
    return () => globalThis.removeEventListener("cowboy:preferences-changed", refresh);
  }, [connected, load]);
  const save = async (days: number): Promise<void> => {
    if (!data) throw new Error("Trash settings are still loading. Try again.");
    setSaving(true);
    try {
      const response = await fetch("/api/preferences", {
        method: "PUT",
        signal: productSessionSignal(),
        headers: { "Content-Type": "application/json", "x-cowboy-dataset": data.dataset.dataset_id },
        body: JSON.stringify({
          operation: crypto.randomUUID(),
          scope: "trash",
          expected_revision: data.preferences.revision,
          changes: { retention_days: days },
        }),
      });
      if (!response.ok) {
        await load();
        throw new Error(response.status === 409 ? "Trash settings changed elsewhere. Try again." : "Could not save the Trash setting.");
      }
      const result = await response.json() as { preferences: TrashPreferences["preferences"] };
      setData({ ...data, preferences: result.preferences });
    } finally {
      setSaving(false);
    }
  };
  return {
    days: data?.preferences.values.retention_days ?? DEFAULT_TRASH_RETENTION_DAYS,
    saving,
    save,
  };
}

/** Trash rows for the current store snapshot and Draft library. */
export function useTrashTree(
  drafts: readonly DraftMetadata[],
  expanded: ReadonlySet<string>,
  retentionDays: number,
): TrashTree {
  const sessions = useStoreSelector((state) => state.trashedSessions);
  const folders = useStoreSelector((state) => state.trashFolders);
  return useMemo(
    () => buildTrashTree(sessions, folders, drafts, expanded, retentionDays),
    [sessions, folders, drafts, expanded, retentionDays],
  );
}

function dayLabel(days: number): string {
  return days === 1 ? "1 day" : `${String(days)} days`;
}

function deadlineText(at: number): string {
  return new Date(at).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

function rowTitle(row: TrashRow): string {
  if (row.kind === "trash-folder") return row.folder.name;
  if (row.kind === "trash-session") return row.session.title || "Session";
  return row.draft.title || "Draft";
}

function Countdown(
  { trashedAt, retentionDays, now }: { trashedAt: number | null; retentionDays: number; now: number },
): React.JSX.Element | null {
  if (trashedAt === null) return null;
  const left = Math.max(0, trashDeadline(trashedAt, retentionDays) - now);
  const urgent = left < TRASH_URGENT_MS;
  return (
    <Typography
      component="span"
      variant="caption"
      aria-label={`Deleted permanently in ${describeTrashRemaining(left)}`}
      title={`Deleted permanently on ${deadlineText(trashDeadline(trashedAt, retentionDays))}`}
      sx={{
        flexShrink: 0,
        fontVariantNumeric: "tabular-nums",
        px: 0.75,
        borderRadius: "999px",
        lineHeight: "1.25rem",
        color: urgent ? "warning.main" : "text.secondary",
        bgcolor: (theme) => urgent ? `${theme.palette.warning.main}1f` : theme.palette.action.hover,
      }}
    >
      {formatTrashRemaining(left)}
    </Typography>
  );
}

export interface SessionTrashSectionProps {
  readonly tree: TrashTree;
  readonly expanded: ReadonlySet<string>;
  readonly onToggle: (key: string) => void;
  readonly retentionDays: number;
  readonly now: number;
  readonly desktop: boolean;
  /** The list's own row geometry (indent, guides, Desktop item chrome). */
  readonly rowSx: (depth: number) => SxProps<Theme>;
  readonly onOpenRow: (row: TrashRow) => void;
  readonly onOpenMenu: () => void;
}

/** The Trash container: one header row, then its rows while open. */
export function SessionTrashSection(
  { tree, expanded, onToggle, retentionDays, now, desktop, rowSx, onOpenRow, onOpenMenu }: SessionTrashSectionProps,
): React.JSX.Element {
  const open = expanded.has(TRASH_ROOT_KEY);
  const iconSx = { fontSize: "1.25rem", flexShrink: 0, color: "text.secondary" } as const;
  return (
    <>
        <ReliableListItemButton
          data-desktop-item={TRASH_ROOT_KEY}
          data-session-trash-root
          aria-expanded={open}
          onActivate={(): void => onToggle(TRASH_ROOT_KEY)}
          sx={[rowSx(0) as object, { mt: 0.5, borderTop: 1, borderColor: "divider" }]}
        >
          <Box className="cowboy-session-grip cowboy-folder-chevron" aria-hidden sx={{ width: desktopSize(44), height: desktopSize(44), flexShrink: 0, display: "grid", placeItems: "center", color: "text.secondary" }}>
            {open ? <ExpandMore sx={{ fontSize: "1.5rem" }} /> : <ChevronRight sx={{ fontSize: "1.5rem" }} />}
          </Box>
          <ListItemText
            primary={
              <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
                <DeleteOutline fontSize="medium" sx={{ flexShrink: 0, color: "text.secondary" }} />
                <Typography variant="body2" noWrap sx={{ minWidth: 0, fontWeight: 600 }}>Trash</Typography>
                {tree.entries > 0 && (
                  <Typography variant="caption" color="text.secondary" sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}>
                    {tree.entries}
                  </Typography>
                )}
              </Stack>
            }
            slotProps={{ primary: { component: "div" } }}
          />
          {!open && tree.soonest !== null && tree.soonest - now < TRASH_URGENT_MS && (
            <Typography variant="caption" color="warning.main" sx={{ flexShrink: 0, mr: 0.5 }}>
              {formatTrashRemaining(Math.max(0, tree.soonest - now))}
            </Typography>
          )}
          <IconButton
            className="cowboy-session-actions"
            aria-label="Trash actions"
            onClick={(event): void => {
              event.stopPropagation();
              onOpenMenu();
            }}
            sx={{ ml: 0.5, width: desktopSize(44), height: desktopSize(44), flexShrink: 0 }}
          >
            <MoreVert sx={{ fontSize: "1.5rem" }} />
          </IconButton>
        </ReliableListItemButton>
        {open && tree.rows.length === 0 && (
          <Box sx={[rowSx(1) as object, { py: 1, color: "text.disabled" }]}>
            <Typography variant="caption" sx={{ fontStyle: "italic" }}>
              Trash is empty. Deleted sessions and folders stay here for {dayLabel(retentionDays)}.
            </Typography>
          </Box>
        )}
        {open && tree.rows.map((row) => (
          <ReliableListItemButton
            key={row.key}
            data-desktop-item={row.key}
            aria-expanded={row.kind === "trash-folder" ? row.expanded : undefined}
            onActivate={(): void => {
              if (row.kind === "trash-folder") onToggle(row.key);
              else onOpenRow(row);
            }}
            sx={[rowSx(row.depth + 1) as object, { opacity: 0.88 }]}
          >
            <Box className="cowboy-session-grip cowboy-folder-chevron" aria-hidden sx={{ width: desktopSize(44), height: desktopSize(desktop ? 32 : 44), flexShrink: 0, display: "grid", placeItems: "center", color: "text.secondary" }}>
              {row.kind === "trash-folder"
                ? row.expanded ? <ExpandMore sx={{ fontSize: "1.5rem" }} /> : <ChevronRight sx={{ fontSize: "1.5rem" }} />
                : null}
            </Box>
            <ListItemText
              primary={
                <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
                  {row.kind === "trash-folder"
                    ? <FolderOutlined sx={iconSx} />
                    : row.kind === "trash-draft"
                    ? <DescriptionOutlined sx={iconSx} />
                    : <ChatBubbleOutline sx={iconSx} />}
                  <Typography variant="body2" noWrap sx={{ minWidth: 0, color: "text.secondary" }}>
                    {rowTitle(row)}
                  </Typography>
                  {row.kind === "trash-folder" && (
                    <Typography variant="caption" color="text.disabled" sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}>
                      {row.itemCount}
                    </Typography>
                  )}
                </Stack>
              }
              secondary={row.origin ? `from ${row.origin}` : undefined}
              slotProps={{
                primary: { component: "div" },
                secondary: { noWrap: true, sx: { fontSize: "0.75rem" } },
              }}
            />
            {!row.sharesDeadline && <Countdown trashedAt={row.trashedAt} retentionDays={retentionDays} now={now} />}
            <IconButton
              className="cowboy-session-actions"
              aria-label={`Trash item actions ${rowTitle(row)}`}
              onClick={(event): void => {
                event.stopPropagation();
                onOpenRow(row);
              }}
              sx={{ ml: 0.5, width: desktopSize(44), height: desktopSize(44), flexShrink: 0 }}
            >
              <MoreVert sx={{ fontSize: "1.5rem" }} />
            </IconButton>
          </ReliableListItemButton>
        ))}
    </>
  );
}

/** The live folder an item inside a trashed folder returns to. */
function nearestLiveFolder(value: SessionFoldersValue, id: string | null): string | null {
  let cursor = id;
  const seen = new Set<string>();
  while (cursor && !seen.has(cursor)) {
    seen.add(cursor);
    const folder = sessionFolderById(value, cursor);
    if (!folder) return null;
    if (folder.trashed_at_ms === undefined) return folder.id;
    cursor = folder.parent;
  }
  return null;
}

/** Restore one Trash row. Resolves once the Controller (or Draft store) agrees. */
export async function restoreTrashRow(row: TrashRow, folders: SessionFoldersValue): Promise<void> {
  if (row.kind === "trash-folder") return restoreItems({ folders: [row.folder.id] });
  if (row.kind === "trash-session") return restoreItems({ sessions: [row.session.id] });
  const owner = draftRepository().document(row.draft.id);
  await owner.hydrate();
  if (row.draft.deleted) await owner.change({ type: "restore" });
  const parent = row.draft.parent_id ?? null;
  const target = nearestLiveFolder(folders, parent);
  if (target !== parent) await owner.change({ type: "move", parent_id: target });
}

/** Actions for one Trash entry: Restore (and open), or delete permanently
 *  after a second, explicit confirmation. */
export function TrashItemSheet(
  { row, retentionDays, now, onClose, onOpenSession }: {
    row: TrashRow;
    retentionDays: number;
    now: number;
    onClose: () => void;
    onOpenSession: (id: string) => void;
  },
): React.JSX.Element {
  const folders = useStoreSelector((state) => state.trashFolders);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const run = (action: () => Promise<void>, done?: () => void): void => {
    setBusy(true);
    void action().then(() => {
      onClose();
      done?.();
    }).catch((error: unknown) => {
      documentNotice(error instanceof Error ? error.message : "Could not finish. Try again.");
    }).finally(() => setBusy(false));
  };
  const restore = (): void => run(() => restoreTrashRow(row, folders), () => documentNotice(`Restored "${rowTitle(row)}"`));
  const purge = (): void =>
    run(
      () => row.kind === "trash-folder"
        ? purgeItems({ folders: [row.folder.id] })
        : row.kind === "trash-session"
        ? purgeItems({ sessions: [row.session.id] })
        : Promise.reject(new Error("Drafts are deleted from Draft history; restore it to edit or copy it.")),
      () => documentNotice(`Deleted "${rowTitle(row)}" permanently`),
    );
  useConfirmEnter(!busy, confirming ? purge : restore);
  const deadline = row.trashedAt === null ? null : trashDeadline(row.trashedAt, retentionDays);
  // An item inside a trashed folder restores alone to the nearest live folder.
  const parentTrashed = row.origin === null;
  const alone = parentTrashed
    ? sessionFolderLocation(folders, nearestLiveFolder(
      folders,
      row.kind === "trash-folder"
        ? row.folder.parent
        : row.kind === "trash-draft"
        ? row.draft.parent_id ?? null
        : folders.placement[row.session.id] ?? null,
    ))
    : null;
  return (
    <InAppRoot>
      <ConfirmSheet
        portal={false}
        open
        onClose={onClose}
        title={confirming ? `Delete "${rowTitle(row)}" permanently?` : rowTitle(row)}
        actions={confirming
          ? (
            <>
              <Button onClick={(): void => setConfirming(false)} color="inherit" disabled={busy}>
                Back
                <Kbd keys="Esc" />
              </Button>
              <Button onClick={purge} color="error" variant="contained" disabled={busy} startIcon={<DeleteForeverOutlined />}>
                Delete permanently
                <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
              </Button>
            </>
          )
          : (
            <>
              <Button onClick={onClose} color="inherit" disabled={busy}>
                Close
                <Kbd keys="Esc" />
              </Button>
              <Button onClick={restore} variant="contained" disabled={busy} startIcon={<RestoreFromTrashOutlined />}>
                Restore
                <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
              </Button>
            </>
          )}
      >
        {confirming
          ? (
            <Typography variant="body2" color="text.secondary">
              {row.kind === "trash-folder"
                ? `The folder, its subfolders and the ${String(row.itemCount)} items inside are deleted now. Drafts inside stay recoverable from Trash. This cannot be undone.`
                : "Its transcript, worktree and agent history are deleted now. This cannot be undone."}
            </Typography>
          )
          : (
            <Stack spacing={1}>
              <Typography variant="body2" color="text.secondary">
                {row.trashedAt !== null ? `Deleted ${describeTrashAge(now - row.trashedAt)}` : "In Trash"}
                {row.origin ? ` from ${row.origin}` : parentTrashed ? " with its folder" : ""}.
              </Typography>
              {deadline !== null && (
                <Typography variant="body2" color={deadline - now < TRASH_URGENT_MS ? "warning.main" : "text.secondary"}>
                  Deleted permanently in {describeTrashRemaining(Math.max(0, deadline - now))} ({deadlineText(deadline)}).
                </Typography>
              )}
              {row.kind === "trash-session" && (
                <Typography variant="caption" color="text.secondary">
                  {sessionListProjectLabel(row.session)} · {row.session.machine_id}. It stays asleep after restore and resumes when you send it work.
                </Typography>
              )}
              {row.kind === "trash-folder" && (
                <Typography variant="caption" color="text.secondary">
                  Restore brings back the folder with its {String(row.itemCount)} items and subfolders.
                </Typography>
              )}
              {alone !== null && (
                <Typography variant="caption" color="text.secondary">
                  Its folder is in Trash too, so restoring only this item puts it in {alone}.
                </Typography>
              )}
              <Stack direction="row" spacing={1} sx={{ pt: 0.5, flexWrap: "wrap" }}>
                {row.kind === "trash-session" && (
                  <Button
                    size="small"
                    disabled={busy}
                    onClick={(): void => run(() => restoreTrashRow(row, folders), () => onOpenSession(row.session.id))}
                  >
                    Restore and open
                  </Button>
                )}
                {row.kind !== "trash-draft" && (
                  <Button size="small" color="error" disabled={busy} onClick={(): void => setConfirming(true)}>
                    Delete permanently…
                  </Button>
                )}
              </Stack>
            </Stack>
          )}
      </ConfirmSheet>
    </InAppRoot>
  );
}

/** Trash-wide actions: retention and Empty Trash. */
export function TrashMenuSheet(
  { tree, retention, onClose }: {
    tree: TrashTree;
    retention: ReturnType<typeof useTrashRetention>;
    onClose: () => void;
  },
): React.JSX.Element {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [days, setDays] = useState(retention.days);
  useEffect(() => setDays(retention.days), [retention.days]);
  // Top-level trashed folders take their subtrees; every trashed session is
  // named so one outside any folder goes too.
  const rootFolders = tree.folders
    .filter((folder) => !tree.folders.some((other) => other.id === folder.parent))
    .map((folder) => folder.id);
  const empty = (): void => {
    setBusy(true);
    void purgeItems({ sessions: tree.sessions.map((session) => session.id), folders: rootFolders })
      .then(() => {
        onClose();
        documentNotice("Trash emptied");
      })
      .catch((error: unknown) => documentNotice(error instanceof Error ? error.message : "Could not empty Trash."))
      .finally(() => setBusy(false));
  };
  const saveDays = (value: number): void => {
    setDays(value);
    void retention.save(value).catch((error: unknown) => {
      setDays(retention.days);
      documentNotice(error instanceof Error ? error.message : "Could not save the Trash setting.");
    });
  };
  useConfirmEnter(confirming && !busy, empty);
  const count = tree.sessions.length;
  return (
    <InAppRoot>
      <ConfirmSheet
        portal={false}
        open
        onClose={onClose}
        title={confirming ? "Empty Trash?" : "Trash"}
        actions={confirming
          ? (
            <>
              <Button onClick={(): void => setConfirming(false)} color="inherit" disabled={busy}>
                Back
                <Kbd keys="Esc" />
              </Button>
              <Button onClick={empty} color="error" variant="contained" disabled={busy} startIcon={<DeleteForeverOutlined />}>
                Delete permanently
                <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
              </Button>
            </>
          )
          : (
            <Button onClick={onClose} color="inherit">
              Close
              <Kbd keys="Esc" />
            </Button>
          )}
      >
        {confirming
          ? (
            <Typography variant="body2" color="text.secondary">
              {`${String(count)} sessions and ${String(tree.folders.length)} folders are deleted now, with their transcripts and worktrees. Drafts stay recoverable from Trash. This cannot be undone.`}
            </Typography>
          )
          : (
            <Stack spacing={2}>
              <TextField
                select
                label="Keep deleted items for"
                value={days}
                disabled={retention.saving}
                onChange={(event): void => saveDays(Number(event.target.value))}
                helperText="Applies to everything in Trash, including items already there. Every device and Cowboy agents share this setting."
                size="small"
              >
                {[...new Set([...TRASH_RETENTION_CHOICES, retention.days])].sort((a, b) => a - b).map((value) => (
                  <MenuItem key={value} value={value}>
                    {dayLabel(value)}{value === DEFAULT_TRASH_RETENTION_DAYS ? " (default)" : ""}
                  </MenuItem>
                ))}
              </TextField>
              <Button
                color="error"
                startIcon={<DeleteForeverOutlined />}
                disabled={tree.entries === 0 || busy}
                onClick={(): void => setConfirming(true)}
                sx={{ alignSelf: "flex-start" }}
              >
                Empty Trash…
              </Button>
            </Stack>
          )}
      </ConfirmSheet>
    </InAppRoot>
  );
}
