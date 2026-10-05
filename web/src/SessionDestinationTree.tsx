import { useRef, useState } from "react";
import {
  Box,
  Chip,
  List,
  ListItemText,
  TextField,
  Typography,
} from "@mui/material";
import { ChevronRight, ExpandMore, FolderOutlined } from "@mui/icons-material";
import type { SessionMeta } from "./protocol";
import { buildSessionTree, displayedSessionOrder } from "./sessionTree";
import {
  folderAncestors,
  sessionFolderLocation,
  type SessionFoldersValue,
} from "./sessionFolders";
import { sessionMachinePresentation } from "./sessionExecution";
import { sessionListProjectLabel } from "./sessionProject";
import { ProviderIcon } from "./ProviderIcon";
import { ReliableListItemButton } from "./ReliableListItemButton";
import { isImeKeyEvent } from "./imeKey";
import { useDialogFocus } from "./useDialogInputFocus";
import { useSurfaceProfile } from "./surface/SurfaceProfile";

/** Session leaves use the same folder projection/order as workspace navigation. */
export function SessionDestinationTree(
  { sessions, folders, order, initialFolder, busy, onPick }: {
    sessions: readonly SessionMeta[];
    folders: SessionFoldersValue;
    order: readonly string[];
    initialFolder: string | null;
    busy: boolean;
    onPick: (session: SessionMeta) => void;
  },
): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  useDialogFocus(() => desktop ? searchRef.current : null, desktop);
  const [query, setQuery] = useState("");
  const [focusId, setFocusId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(() => {
    const open = new Set([
      initialFolder,
      ...folderAncestors(folders, initialFolder),
    ]);
    return new Set(
      folders.folders.filter((folder) => !open.has(folder.id)).map((folder) =>
        folder.id
      ),
    );
  });
  const eligible = displayedSessionOrder(
    sessions.filter((session) => !session.system),
  );
  const full = buildSessionTree(eligible, folders, new Set(), [], order);
  const needle = query.trim().toLocaleLowerCase();
  const matches = eligible.filter((session) =>
    `${session.title} ${
      sessionFolderLocation(folders, full.folderOf.get(session.id) ?? null)
    } ${sessionListProjectLabel(session)} ${
      sessionMachinePresentation(session).label
    }`.toLocaleLowerCase().includes(needle)
  );
  const rows = buildSessionTree(
    needle ? matches : eligible,
    folders,
    needle ? new Set() : collapsed,
    [],
    order,
  ).rows
    .filter((row) =>
      row.kind === "session" || row.kind === "folder" && row.sessionCount > 0
    );
  const visibleFocus =
    rows.some((row) =>
        (row.kind === "folder"
          ? `folder:${row.folder.id}`
          : row.kind === "session"
          ? `session:${row.session.id}`
          : null) === focusId
      )
      ? focusId
      : null;
  const toggle = (id: string) =>
    setCollapsed((old) => {
      const next = new Set(old);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const targets = () => [
    ...listRef.current?.querySelectorAll<HTMLElement>(
      "[data-session-destination-row]:not([aria-disabled=true])",
    ) ?? [],
  ];
  return (
    <Box sx={{ minHeight: 0, minWidth: 0 }}>
      <TextField
        inputRef={searchRef}
        label="Find Session"
        value={query}
        fullWidth
        size="small"
        onChange={(event) => setQuery(event.target.value)}
        sx={{ mt: 1, mb: 1.5 }}
        onKeyDown={(event) => {
          if (busy || isImeKeyEvent(event.nativeEvent)) return;
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            const buttons = targets();
            (event.key === "ArrowUp" ? buttons.at(-1) : buttons[0])?.focus();
          }
        }}
      />
      <Box
        sx={{ overflowY: "auto", maxHeight: "min(55dvh, 32rem)", minHeight: 0 }}
      >
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ display: "block", px: 1, py: 0.5 }}
        >
          Top level
        </Typography>
        <List
          ref={listRef}
          role="tree"
          aria-label="Destination Sessions"
          disablePadding
          onKeyDownCapture={(event) => {
            if (isImeKeyEvent(event.nativeEvent)) event.stopPropagation();
          }}
          onKeyDown={(event) => {
            if (busy || isImeKeyEvent(event.nativeEvent)) return;
            const buttons = targets();
            const current = document.activeElement as HTMLElement;
            const index = buttons.indexOf(current);
            if (index < 0) return;
            const row = rows[index];
            if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
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
            } else if (event.key === "ArrowRight" && row?.kind === "folder") {
              event.preventDefault();
              if (!row.expanded && !needle) toggle(row.folder.id);
              else if (rows[index + 1] && rows[index + 1]!.depth > row.depth) {
                buttons[index + 1]?.focus();
              }
            } else if (event.key === "ArrowLeft" && row) {
              event.preventDefault();
              if (row.kind === "folder" && row.expanded && !needle) {
                toggle(row.folder.id);
              } else {
                for (let parent = index - 1; parent >= 0; parent--) {
                  if (rows[parent]!.depth < row.depth) {
                    buttons[parent]?.focus();
                    break;
                  }
                }
              }
            }
          }}
        >
          {rows.map((row, index) => {
            if (row.kind !== "session" && row.kind !== "folder") return null;
            const folder = row.kind === "folder";
            const id = folder
              ? `folder:${row.folder.id}`
              : `session:${row.session.id}`;
            const machine = folder
              ? null
              : sessionMachinePresentation(row.session);
            return (
              <ReliableListItemButton
                key={id}
                role="treeitem"
                data-session-destination-row={id}
                data-session-destination-folder={folder
                  ? row.folder.id
                  : undefined}
                data-session-destination-session={!folder
                  ? row.session.id
                  : undefined}
                title={folder
                  ? row.folder.name
                  : `${
                    sessionFolderLocation(folders, row.folder)
                  }\n${row.session.cwd}\n${machine?.description ?? ""}`}
                aria-level={row.depth + 1}
                aria-expanded={folder ? row.expanded : undefined}
                tabIndex={visibleFocus === id || (!visibleFocus && index === 0)
                  ? 0
                  : -1}
                disabled={busy}
                onFocus={() => setFocusId(id)}
                onActivate={() => {
                  if (busy) return;
                  if (folder) { if (!needle) toggle(row.folder.id); }
                  else onPick(row.session);
                }}
                sx={{
                  pl: `min(${0.5 + Math.min(row.depth, 8) * 1.25}rem, 25%)`,
                  pr: "0.75rem",
                  gap: "0.5rem",
                  minHeight: desktop ? "2.75rem" : 48,
                  py: "0.375rem",
                  "& .MuiSvgIcon-root": { fontSize: "1.25rem", flexShrink: 0 },
                }}
              >
                <Box
                  component="span"
                  sx={{ width: "1.25rem", flexShrink: 0, display: "flex" }}
                >
                  {folder
                    ? row.expanded ? <ExpandMore /> : <ChevronRight />
                    : null}
                </Box>
                {folder ? <FolderOutlined /> : (
                  <ProviderIcon
                    provider={row.session.provider}
                    providerVersion={row.session.provider_version}
                    providerDigest={row.session.provider_generation_digest}
                  />
                )}
                <ListItemText
                  primary={folder
                    ? row.folder.name
                    : row.session.title || "Session"}
                  secondary={!folder
                    ? sessionListProjectLabel(row.session)
                    : undefined}
                  sx={{ minWidth: 0, my: 0 }}
                  slotProps={{
                    primary: {
                      noWrap: true,
                      fontWeight: folder ? 600 : undefined,
                    },
                    secondary: { noWrap: true },
                  }}
                />
                {folder
                  ? (
                    <Typography variant="caption" color="text.secondary">
                      {row.sessionCount}
                    </Typography>
                  )
                  : machine?.visible
                  ? (
                    <Chip
                      size="small"
                      variant="outlined"
                      label={machine.label}
                      title={machine.description}
                      sx={{ maxWidth: "35%" }}
                    />
                  )
                  : null}
              </ReliableListItemButton>
            );
          })}
        </List>
        {matches.length === 0 && (
          <Typography role="status" color="text.secondary" sx={{ p: 2 }}>
            {needle
              ? "No matching Sessions"
              : "Create a Session to add this draft."}
          </Typography>
        )}
      </Box>
    </Box>
  );
}
