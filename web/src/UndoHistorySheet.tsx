// Undo history: the changes this device can still (or could) undo, newest
// first. Opened from a notice's history button, the Trash menu, the Desktop
// command palette and `␣I`. See docs/undo-history.md.
import {
  ChatBubbleOutline,
  DescriptionOutlined,
  FolderOutlined,
  History,
  SmartToyOutlined,
} from "@mui/icons-material";
import {
  Box,
  Button,
  CircularProgress,
  Stack,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { desktopRecentAge } from "./desktop/commands/DesktopRecentDialog";
import { documentNotice } from "./documents/DocumentNotifications";
import { ConfirmSheet } from "./Sheet";
import { useStoreSelector } from "./store";
import { undoEntry } from "./undoActions";
import {
  effectiveUndoState,
  pruneUndoHistory,
  readUndoHistory,
  subscribeUndoHistory,
  trashUndoBlocker,
  type TrashView,
  UNDO_TTL_MS,
  type UndoEntry,
  type UndoState,
} from "./undoHistory";

let open = false;
const openListeners = new Set<() => void>();
function setOpen(next: boolean): void {
  open = next;
  for (const listener of openListeners) listener();
}
export function openUndoHistory(): void {
  setOpen(true);
}
const subscribeOpen = (listener: () => void): () => void => {
  openListeners.add(listener);
  return () => {
    openListeners.delete(listener);
  };
};
const readOpen = (): boolean => open;

/** `just now`, `5m ago`, `3h ago`. */
function ago(at: number, now: number): string {
  const age = desktopRecentAge(at, now);
  return age === "now" ? "just now" : `${age} ago`;
}

const STATE_LABEL: Record<UndoState, string> = {
  available: "Can undo",
  undoing: "Undoing…",
  used: "Undone",
  unavailable: "Unavailable",
};

function EntryIcon({ entry }: { entry: UndoEntry }): React.JSX.Element {
  const sx = { fontSize: "1.25rem", color: "text.secondary", mt: "2px" };
  const step = entry.steps?.[0];
  if (step?.kind === "restore_trash") {
    return step.folders?.length ? <FolderOutlined sx={sx} /> : <ChatBubbleOutline sx={sx} />;
  }
  if (step?.kind === "restore_calls") return <SmartToyOutlined sx={sx} />;
  if (step) return <DescriptionOutlined sx={sx} />;
  return <History sx={sx} />;
}

function StateBadge(
  { state }: { state: UndoState },
): React.JSX.Element {
  return (
    <Typography
      component="span"
      variant="caption"
      data-undo-state={state}
      sx={{
        flexShrink: 0,
        px: 0.75,
        borderRadius: "999px",
        lineHeight: "1.25rem",
        // Tinted, borderless badges: neutral for settled entries, primary for
        // the one state that still offers an action.
        color: state === "available" ? "primary.main" : "text.secondary",
        bgcolor: (theme) =>
          state === "available"
            ? `${theme.palette.primary.main}1f`
            : theme.palette.action.hover,
      }}
    >
      {STATE_LABEL[state]}
    </Typography>
  );
}

function useTrashView(): TrashView {
  const ready = useStoreSelector((state) => state.sessionsLoaded && state.sessionFoldersReady);
  const live = useStoreSelector((state) => state.sessions);
  const trashed = useStoreSelector((state) => state.trashedSessions);
  const folders = useStoreSelector((state) => state.trashFolders);
  return useMemo(() => ({
    ready,
    liveSessions: new Set(live.map((session) => session.id)),
    trashedSessions: new Set(trashed.map((session) => session.id)),
    folders: new Map(folders.folders.map((folder) => [folder.id, folder.trashed_at_ms !== undefined])),
  }), [ready, live, trashed, folders]);
}

/** Mounted once beside the notices. */
export function UndoHistoryHost(): React.JSX.Element | null {
  const isOpen = useSyncExternalStore(subscribeOpen, readOpen, readOpen);
  return isOpen ? <UndoHistorySheet onClose={() => setOpen(false)} /> : null;
}

function UndoHistorySheet({ onClose }: { onClose: () => void }): React.JSX.Element {
  const entries = useSyncExternalStore(subscribeUndoHistory, readUndoHistory, readUndoHistory);
  const trash = useTrashView();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    pruneUndoHistory();
    const timer = globalThis.setInterval(() => setNow(Date.now()), 30_000);
    return () => globalThis.clearInterval(timer);
  }, []);
  const hours = Math.round(UNDO_TTL_MS / 3_600_000);
  return (
    <ConfirmSheet open onClose={onClose} title="Undo history">
      <Stack spacing={1.5} data-undo-history>
        <Typography variant="body2" color="text.secondary">
          Changes made on this device in the last {hours} hours. Older entries are cleared automatically; deleted items stay in Trash longer.
        </Typography>
        {entries.length === 0
          ? (
            <Typography variant="body2" color="text.disabled" sx={{ py: 2, textAlign: "center", fontStyle: "italic" }}>
              Nothing to undo yet.
            </Typography>
          )
          : (
            <Stack component="ul" sx={{ listStyle: "none", m: 0, p: 0 }} divider={<Box component="li" aria-hidden sx={{ borderTop: 1, borderColor: "divider" }} />}>
              {entries.map((entry) => {
                const effective = effectiveUndoState(entry);
                const blocker = effective.state === "available" ? trashUndoBlocker(entry.steps, trash) : undefined;
                const state: UndoState = blocker ? "unavailable" : effective.state;
                const reason = blocker ?? effective.reason;
                const detail = state === "used" && entry.settledAt
                  ? `Undone ${ago(entry.settledAt, now)}`
                  : state === "unavailable" && reason
                  ? reason
                  : undefined;
                return (
                  <Stack
                    key={entry.id}
                    component="li"
                    direction="row"
                    spacing={1.25}
                    alignItems="flex-start"
                    data-undo-entry={entry.id}
                    sx={{ py: 1, opacity: state === "available" || state === "undoing" ? 1 : 0.72 }}
                  >
                    <EntryIcon entry={entry} />
                    <Stack sx={{ flex: 1, minWidth: 0 }} spacing={0.25}>
                      <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>{entry.message}</Typography>
                      <Stack direction="row" spacing={0.75} alignItems="center" flexWrap="wrap" useFlexGap>
                        <StateBadge state={state} />
                        <Typography variant="caption" color="text.secondary" sx={{ fontVariantNumeric: "tabular-nums" }}>
                          {ago(entry.createdAt, now)}
                        </Typography>
                      </Stack>
                      {detail && (
                        <Typography variant="caption" color="text.secondary" sx={{ overflowWrap: "anywhere" }}>
                          {detail}
                        </Typography>
                      )}
                    </Stack>
                    {state === "available" && (
                      <Button
                        size="small"
                        onClick={() =>
                          void undoEntry(entry.id).catch((error: unknown) =>
                            documentNotice(error instanceof Error ? error.message : "Could not undo")
                          )}
                        sx={{ flexShrink: 0 }}
                      >
                        Undo
                      </Button>
                    )}
                    {state === "undoing" && <CircularProgress size={18} sx={{ flexShrink: 0, mt: 0.5, mr: 1 }} />}
                  </Stack>
                );
              })}
            </Stack>
          )}
      </Stack>
    </ConfirmSheet>
  );
}
