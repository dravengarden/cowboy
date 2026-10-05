import { isImeKeyEvent } from "../imeKey";
import { useMemo, useRef, useState } from "react";
import {
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  TextField,
  Typography,
} from "@mui/material";
import { ChevronRight, ExpandMore, FolderOutlined } from "@mui/icons-material";
import type { SessionMeta } from "../protocol";
import { useStoreSelector } from "../store";
import { buildSessionTree, displayedSessionOrder } from "../sessionTree";
import {
  sessionFolderLocation,
  type SessionFoldersValue,
} from "../sessionFolders";
import {
  sessionDisplayDirectory,
  sessionListProjectLabel,
} from "../sessionProject";
import { sessionMachinePresentation } from "../sessionExecution";
import { ProviderIcon } from "../ProviderIcon";
import { useDialogInputFocus } from "../useDialogInputFocus";
import {
  desktopModalBackdropSx,
  desktopModalPaperSx,
} from "./DesktopEmbeddedControl";

/** Mounted only while the Desktop picker is open: live list updates never
 * subscribe the writing canvas to unrelated session status or folder changes. */
export function DesktopDraftDestinationPicker(props: {
  sourceId: string;
  onPick: (session: SessionMeta) => void;
  onClose: () => void;
}) {
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const folders = useStoreSelector((snapshot) => snapshot.sessionFolders);
  return (
    <DraftDestinationDialog {...props} sessions={sessions} folders={folders} />
  );
}

export function DraftDestinationDialog(
  { sessions, folders, sourceId, onPick, onClose }: {
    sessions: readonly SessionMeta[];
    folders: SessionFoldersValue;
    sourceId: string;
    onPick: (session: SessionMeta) => void;
    onClose: () => void;
  },
) {
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  useDialogInputFocus(searchRef, true);
  const eligible = useMemo(
    () => displayedSessionOrder(sessions.filter((s) => s.id !== sourceId)),
    [sessions, sourceId],
  );
  const fullTree = useMemo(
    () => buildSessionTree(eligible, folders, new Set()),
    [eligible, folders],
  );
  const needle = query.trim().toLocaleLowerCase();
  const matches = eligible.filter((s) => {
    const path = sessionFolderLocation(
      folders,
      fullTree.folderOf.get(s.id) ?? null,
    );
    const machine = sessionMachinePresentation(s);
    return `${s.title} ${path} ${sessionListProjectLabel(s)} ${
      sessionDisplayDirectory(s)
    } ${s.cwd} ${machine.label} ${s.provider}`
      .toLocaleLowerCase().includes(needle);
  });
  // Search opens ancestor paths without changing the user's local disclosure.
  const tree = buildSessionTree(
    needle ? matches : eligible,
    folders,
    needle ? new Set() : collapsed,
  );
  const rows = tree.rows.filter((row) =>
    !needle ||
    (row.kind === "session" || row.kind === "folder" && row.sessionCount > 0)
  );
  const toggleFolder = (id: string) =>
    setCollapsed((old) => {
      const next = new Set(old);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const focusEdge = (last = false) => {
    const buttons = listRef.current?.querySelectorAll<HTMLElement>(
      "[data-draft-destination-row]:not([aria-disabled='true'])",
    );
    (last ? buttons?.[buttons.length - 1] : buttons?.[0])?.focus();
  };
  return (
    <Dialog
      open
      onKeyDownCapture={(event) => {
        if (isImeKeyEvent(event.nativeEvent)) event.stopPropagation();
      }}
      onClose={onClose}
      fullWidth
      maxWidth={false}
      slotProps={{
        paper: {
          sx: {
            ...desktopModalPaperSx(),
            width: "min(640px, calc(100vw - 32px))",
            m: 2,
            maxHeight: "calc(100vh - 32px)",
          },
        },
        backdrop: { sx: desktopModalBackdropSx() },
      }}
    >
      <DialogTitle>Move draft to a session</DialogTitle>
      <Box sx={{ px: 3, pb: 1.5 }}>
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
          Choose a session inside a folder or at Global. The draft stays unsent.
        </Typography>
        <TextField
          inputRef={searchRef}
          autoFocus
          fullWidth
          size="small"
          value={query}
          placeholder="Search sessions, folders or projects…"
          slotProps={{
            htmlInput: { "aria-label": "Search draft destinations" },
          }}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (isImeKeyEvent(event.nativeEvent)) return;
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              focusEdge(event.key === "ArrowUp");
            }
          }}
        />
      </Box>
      <DialogContent
        dividers
        sx={{
          p: 0,
          minHeight: 0,
          height: "min(440px, 55vh)",
          overflowY: "auto",
        }}
      >
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ display: "block", px: 3, py: 1 }}
        >
          Global
        </Typography>
        <List
          ref={listRef}
          disablePadding
          aria-label="Draft destination sessions"
          onKeyDown={(event) => {
            if (isImeKeyEvent(event.nativeEvent)) return;
            if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
              return;
            }
            const buttons = Array.from(
              listRef.current?.querySelectorAll<HTMLElement>(
                "[data-draft-destination-row]:not([aria-disabled='true'])",
              ) ?? [],
            );
            const index = buttons.indexOf(
              document.activeElement as HTMLElement,
            );
            if (index < 0) return;
            event.preventDefault();
            const next = event.key === "Home"
              ? 0
              : event.key === "End"
              ? buttons.length - 1
              : Math.max(
                0,
                Math.min(
                  buttons.length - 1,
                  index + (event.key === "ArrowDown" ? 1 : -1),
                ),
              );
            buttons[next]?.focus();
          }}
        >
          {rows.map((row) => {
            const inset = Math.min(row.depth, 8) * 2;
            if (row.kind === "empty") {
              return (
                <Typography
                  key={`empty:${row.folder}`}
                  variant="caption"
                  color="text.secondary"
                  sx={{ display: "block", pl: 7 + inset, py: 1 }}
                >
                  No destination sessions
                </Typography>
              );
            }
            if (row.kind === "folder") {
              return (
                <ListItemButton
                  key={`folder:${row.folder.id}`}
                  data-draft-destination-row
                  data-draft-folder={row.folder.id}
                  aria-label={`${
                    row.expanded ? "Collapse" : "Expand"
                  } ${row.folder.name}`}
                  aria-expanded={row.expanded}
                  disabled={Boolean(needle)}
                  onClick={() => toggleFolder(row.folder.id)}
                  onKeyDown={(event) => {
                    if (isImeKeyEvent(event.nativeEvent)) return;
                    if (
                      !needle &&
                      ((event.key === "ArrowLeft" && row.expanded) ||
                        (event.key === "ArrowRight" && !row.expanded))
                    ) {
                      event.preventDefault();
                      toggleFolder(row.folder.id);
                    }
                  }}
                  sx={{ pl: 2 + inset, pr: 3, py: 0.5, minHeight: 36 }}
                >
                  <ListItemIcon sx={{ minWidth: 28 }}>
                    {row.expanded
                      ? <ExpandMore fontSize="small" />
                      : <ChevronRight fontSize="small" />}
                  </ListItemIcon>
                  <FolderOutlined fontSize="small" sx={{ mr: 1 }} />
                  <ListItemText
                    primary={row.folder.name}
                    slotProps={{
                      primary: {
                        variant: "body2",
                        noWrap: true,
                        fontWeight: 600,
                      },
                    }}
                  />
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ ml: 1 }}
                  >
                    {row.sessionCount}
                  </Typography>
                </ListItemButton>
              );
            }
            const s = row.kind === "session" ? row.session : null;
            if (!s) return null;
            const machine = sessionMachinePresentation(s);
            const path = sessionFolderLocation(folders, row.folder);
            return (
              <ListItemButton
                key={s.id}
                data-draft-destination-row
                data-draft-session={s.id}
                onClick={() => onPick(s)}
                title={`${path}\n${sessionDisplayDirectory(s)}\n${s.cwd}`}
                sx={{
                  pl: 5.5 + inset,
                  pr: 3,
                  py: 0.75,
                  minHeight: 52,
                  gap: 1,
                  minWidth: 0,
                }}
              >
                <ProviderIcon
                  provider={s.provider}
                  providerVersion={s.provider_version}
                  providerDigest={s.provider_generation_digest}
                  fontSize="small"
                />
                <ListItemText
                  primary={s.title}
                  secondary={`${sessionListProjectLabel(s)}${
                    needle ? ` · ${path}` : ""
                  }`}
                  sx={{ minWidth: 0 }}
                  slotProps={{
                    primary: { noWrap: true },
                    secondary: { noWrap: true },
                  }}
                />
                {machine.visible && (
                  <Chip
                    size="small"
                    variant="outlined"
                    label={machine.label}
                    title={machine.description}
                    sx={{ maxWidth: "35%", flexShrink: 0 }}
                  />
                )}
              </ListItemButton>
            );
          })}
          {matches.length === 0 && (
            <Typography
              role="status"
              color="text.secondary"
              sx={{ px: 3, py: 3 }}
            >
              {needle ? "No matching sessions" : "No other sessions available"}
            </Typography>
          )}
        </List>
      </DialogContent>
      <DialogActions sx={{ px: 3, py: 1.5 }}>
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ mr: "auto" }}
        >
          ↑↓ Navigate · Enter Choose · Esc Cancel
        </Typography>
        <Button onClick={onClose} color="inherit">Cancel</Button>
      </DialogActions>
    </Dialog>
  );
}
