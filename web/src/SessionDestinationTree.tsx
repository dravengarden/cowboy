import { useEffect, useMemo, useRef, useState } from "react";
import {
  Box,
  Chip,
  InputAdornment,
  List,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import type { SxProps, Theme } from "@mui/material";
import {
  ChevronRight,
  ExpandMore,
  FolderOpenOutlined,
  FolderOutlined,
  LabelOutlined,
  Search,
} from "@mui/icons-material";
import type { SessionMeta } from "./protocol";
import {
  buildSessionTree,
  displayedSessionOrder,
  type FolderRow,
} from "./sessionTree";
import {
  folderAncestors,
  sessionFolderLocation,
  type SessionFoldersValue,
} from "./sessionFolders";
import { sessionMachinePresentation } from "./sessionExecution";
import {
  sessionDisplayDirectory,
  sessionListProjectLabel,
} from "./sessionProject";
import { ProviderIcon } from "./ProviderIcon";
import { ReliableListItemButton } from "./ReliableListItemButton";
import { SessionMachineBadge } from "./SessionMachineBadge";
import { StatusDot } from "./SessionStatusDot";
import { isImeKeyEvent } from "./imeKey";
import { desktopKeyIntent } from "./desktop/commands/keyIntent";
import { useDialogFocus } from "./useDialogInputFocus";
import { useSurfaceProfile } from "./surface/SurfaceProfile";

// Geometry follows the Sessions sidebar (App.tsx SessionList) so a picker
// reads as the same tree: rows indent by MARGIN, one step clears the parent
// chevron's centre, and a hairline guide per ancestor sits under that
// chevron. Touch keeps a 28px chevron, fine pointers a 24px one; both centre
// 18px into the row, so the guide math is shared.
const FINE = "@media (pointer: fine) and (hover: hover)";
const INDENT_STEP = 22;
const GUIDE_X = 18;
const ROW_PL = { touch: 4, fine: 6 } as const;
const CHEVRON = { touch: 28, fine: 24 } as const;
const PREFIX_TIMEOUT_MS = 1200;

const guideSx = (depth: number): SxProps<Theme> => {
  if (depth === 0) return {};
  const levels = Array.from({ length: depth }, (_, level) => level);
  return {
    "&::before": {
      content: '""',
      position: "absolute",
      top: "-2px",
      bottom: "-2px",
      left: `${String(-depth * INDENT_STEP)}px`,
      right: 0,
      pointerEvents: "none",
      backgroundImage: (t: Theme) =>
        levels.map(() =>
          `linear-gradient(${t.palette.divider}, ${t.palette.divider})`
        ).join(", "),
      backgroundSize: levels.map(() => "1px 100%").join(", "),
      backgroundRepeat: "no-repeat",
      backgroundPosition: levels
        .map((level) => `${String(level * INDENT_STEP + GUIDE_X)}px 0`)
        .join(", "),
    },
  };
};

export interface SessionDestinationTreeState {
  /** The search field owns keys (Insert); list motions are inactive. */
  readonly searchFocused: boolean;
  /** `g` is pending its second key. */
  readonly prefixArmed: boolean;
}

/**
 * Pick a Session from the Sessions folder tree. Rows, order, folds and guides
 * mirror the sidebar; the source Session stays visible (disabled) so the
 * user keeps their bearings. Keyboard: J/K or arrows move, H/L fold or walk
 * the tree, gg/G jump, `/` searches, L/Enter chooses; `Esc` in search
 * returns to the list.
 */
export function SessionDestinationTree(
  {
    sessions,
    folders,
    order,
    initialFolder,
    currentId = null,
    busy = false,
    onPick,
    onStateChange,
    listSx,
  }: {
    sessions: readonly SessionMeta[];
    folders: SessionFoldersValue;
    order: readonly string[];
    /** Folder whose path starts open; defaults to the current Session's. */
    initialFolder?: string | null;
    /** The Session the item lives in today: shown, never selectable. */
    currentId?: string | null;
    busy?: boolean;
    onPick: (session: SessionMeta) => void;
    onStateChange?: (state: SessionDestinationTreeState) => void;
    listSx?: SxProps<Theme>;
  },
): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const prefixTimer = useRef<number | null>(null);
  // Desktop opens in search (Insert): typing filters at once, `Esc` or `↓`
  // reaches the tree. Touch never raises the keyboard uninvited.
  useDialogFocus(() => desktop ? searchRef.current : null, desktop);
  const [query, setQuery] = useState("");
  const [focusId, setFocusId] = useState<string | null>(null);
  const [searchFocused, setSearchFocused] = useState(false);
  const [prefixArmed, setPrefixArmed] = useState(false);
  const eligible = useMemo(
    () => displayedSessionOrder(sessions.filter((session) => !session.system)),
    [sessions],
  );
  const full = useMemo(
    () => buildSessionTree(eligible, folders, new Set(), [], order),
    [eligible, folders, order],
  );
  const [collapsed, setCollapsed] = useState(() => {
    const context = initialFolder !== undefined
      ? initialFolder
      : currentId
      ? full.folderOf.get(currentId) ?? null
      : null;
    const open = new Set([context, ...folderAncestors(folders, context)]);
    return new Set(
      folders.folders.filter((folder) => !open.has(folder.id)).map((folder) =>
        folder.id
      ),
    );
  });
  useEffect(() => {
    onStateChange?.({ searchFocused, prefixArmed });
  }, [onStateChange, searchFocused, prefixArmed]);
  useEffect(() => () => {
    if (prefixTimer.current !== null) clearTimeout(prefixTimer.current);
  }, []);
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
  const choosable = matches.some((session) => session.id !== currentId);
  const rowId = (index: number): string | null => {
    const row = rows[index];
    return row?.kind === "folder"
      ? `folder:${row.folder.id}`
      : row?.kind === "session"
      ? `session:${row.session.id}`
      : null;
  };
  const visibleFocus = rows.some((_, index) => rowId(index) === focusId)
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
  const focusTarget = (target: HTMLElement | undefined) => {
    target?.focus({ preventScroll: true });
    target?.scrollIntoView({ block: "nearest" });
  };
  const focusList = (first = false) => {
    const buttons = targets();
    focusTarget(
      first
        ? buttons.find((button) =>
          button.hasAttribute("data-session-destination-session")
        ) ?? buttons[0]
        : buttons.find((button) => button.tabIndex === 0) ?? buttons[0],
    );
  };
  const clearPrefix = () => {
    if (prefixTimer.current !== null) clearTimeout(prefixTimer.current);
    prefixTimer.current = null;
    setPrefixArmed(false);
  };
  const onTreeKeyDown = (event: React.KeyboardEvent<HTMLUListElement>) => {
    if (busy) return;
    const intent = desktopKeyIntent(event.nativeEvent);
    if (intent.owner !== "command") return;
    const buttons = targets();
    const index = buttons.indexOf(document.activeElement as HTMLElement);
    if (index < 0) return;
    const rowIndex = Number(buttons[index]!.dataset.sessionDestinationIndex);
    const row = rows[rowIndex];
    const key = intent.key;
    const handled = () => {
      event.preventDefault();
      event.stopPropagation();
    };
    if (prefixArmed) {
      clearPrefix();
      if (key === "g" && !intent.modified) {
        handled();
        focusTarget(buttons[0]);
        return;
      }
    }
    if (intent.modified) {
      // Ctrl-D/U: half a page, as in every Desktop list.
      const direction = !event.ctrlKey || event.metaKey || event.altKey
        ? 0
        : key === "d"
        ? 1
        : key === "u"
        ? -1
        : 0;
      if (direction === 0) return;
      handled();
      const height = buttons[index]!.getBoundingClientRect().height || 44;
      const page = Math.max(
        1,
        Math.floor(
          (listRef.current?.parentElement?.clientHeight ?? 0) / height / 2,
        ),
      );
      focusTarget(
        buttons[
          Math.max(0, Math.min(buttons.length - 1, index + direction * page))
        ],
      );
      return;
    }
    const parent = () => {
      if (!row) return;
      for (let i = index - 1; i >= 0; i--) {
        const candidate =
          rows[Number(buttons[i]!.dataset.sessionDestinationIndex)];
        if (candidate && candidate.depth < row.depth) {
          focusTarget(buttons[i]);
          return;
        }
      }
    };
    switch (key) {
      case "ArrowDown":
      case "j":
      case "ArrowUp":
      case "k": {
        handled();
        const delta = key === "ArrowDown" || key === "j" ? 1 : -1;
        focusTarget(
          buttons[Math.max(0, Math.min(buttons.length - 1, index + delta))],
        );
        return;
      }
      case "Home":
        handled();
        focusTarget(buttons[0]);
        return;
      case "End":
      case "G":
        handled();
        focusTarget(buttons.at(-1));
        return;
      case "g":
        handled();
        setPrefixArmed(true);
        prefixTimer.current = globalThis.setTimeout(
          clearPrefix,
          PREFIX_TIMEOUT_MS,
        );
        return;
      case "ArrowRight":
      case "l":
        handled();
        if (row?.kind === "session") {
          if (key === "l") onPick(row.session);
        } else if (row?.kind === "folder") {
          if (!row.expanded && !needle) toggle(row.folder.id);
          else if (
            rows[rowIndex + 1] && rows[rowIndex + 1]!.depth > row.depth
          ) {
            focusTarget(buttons[index + 1]);
          }
        }
        return;
      case "ArrowLeft":
      case "h":
        handled();
        if (row?.kind === "folder" && row.expanded && !needle) {
          toggle(row.folder.id);
        } else parent();
        return;
      case "/":
        handled();
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
    }
  };
  return (
    <Stack sx={{ minHeight: 0, minWidth: 0, flex: 1 }}>
      <TextField
        inputRef={searchRef}
        value={query}
        fullWidth
        size="small"
        placeholder="Search sessions…"
        onChange={(event) => setQuery(event.target.value)}
        onFocus={() => setSearchFocused(true)}
        onBlur={() => setSearchFocused(false)}
        sx={{ mb: 1, flexShrink: 0 }}
        slotProps={{
          htmlInput: {
            "aria-label": "Search destination Sessions",
            enterKeyHint: "search",
          },
          input: {
            startAdornment: (
              <InputAdornment position="start">
                <Search fontSize="small" />
              </InputAdornment>
            ),
          },
        }}
        onKeyDown={(event) => {
          if (busy || isImeKeyEvent(event.nativeEvent)) return;
          if (event.key === "Escape" && desktop) {
            // Leave Insert for the tree; a second Esc closes the modal.
            event.preventDefault();
            event.stopPropagation();
            focusList();
          } else if (event.key === "ArrowDown" || event.key === "Enter") {
            event.preventDefault();
            focusList(true);
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            focusTarget(targets().at(-1));
          }
        }}
      />
      <Box
        sx={[
          // The caller decides who scrolls: Desktop gives this box a height,
          // touch sheets let their own body scroll it.
          { minHeight: 0, flex: 1 },
          ...(Array.isArray(listSx) ? listSx : listSx ? [listSx] : []),
        ]}
      >
        <List
          ref={listRef}
          role="tree"
          aria-label="Destination Sessions"
          disablePadding
          onKeyDownCapture={(event) => {
            if (isImeKeyEvent(event.nativeEvent)) event.stopPropagation();
          }}
          onKeyDown={onTreeKeyDown}
        >
          {rows.map((row, index) => {
            if (row.kind !== "session" && row.kind !== "folder") return null;
            const folder = row.kind === "folder";
            const id = rowId(index)!;
            const current = !folder && row.session.id === currentId;
            const firstTarget = !visibleFocus &&
              index === rows.findIndex((r) =>
                  r.kind === "folder" ||
                  (r.kind === "session" && r.session.id !== currentId)
                );
            return (
              <ReliableListItemButton
                key={id}
                role="treeitem"
                data-session-destination-row={id}
                data-session-destination-index={index}
                data-session-destination-folder={folder
                  ? row.folder.id
                  : undefined}
                data-session-destination-session={!folder
                  ? row.session.id
                  : undefined}
                data-session-destination-current={current ? "true" : undefined}
                title={folder
                  ? sessionFolderLocation(folders, row.folder.id)
                  : `${sessionFolderLocation(folders, row.folder)}\n${
                    sessionDisplayDirectory(row.session)
                  }`}
                aria-level={row.depth + 1}
                aria-expanded={folder ? row.expanded : undefined}
                aria-current={current ? "true" : undefined}
                aria-disabled={busy || current || undefined}
                tabIndex={visibleFocus === id || firstTarget ? 0 : -1}
                onFocus={() => setFocusId(id)}
                onActivate={() => {
                  if (busy || current) return;
                  if (folder) { if (!needle) toggle(row.folder.id); }
                  else onPick(row.session);
                }}
                sx={{
                  ...guideSx(row.depth),
                  position: "relative",
                  ml: `${String(row.depth * INDENT_STEP)}px`,
                  pl: `${String(ROW_PL.touch)}px`,
                  pr: 1,
                  my: "2px",
                  gap: 0.75,
                  minHeight: folder ? 44 : 52,
                  py: 0.5,
                  borderRadius: "10px",
                  ...(current && {
                    cursor: "default",
                    "&:hover": { bgcolor: "transparent" },
                  }),
                  ...(busy && { opacity: 0.6, pointerEvents: "none" }),
                  [FINE]: {
                    pl: `${String(ROW_PL.fine)}px`,
                    minHeight: folder ? 32 : 40,
                    py: 0.25,
                  },
                }}
              >
                <Box
                  component="span"
                  aria-hidden={folder || undefined}
                  sx={{
                    width: CHEVRON.touch,
                    flexShrink: 0,
                    display: "grid",
                    placeItems: "center",
                    color: "text.secondary",
                    [FINE]: { width: CHEVRON.fine },
                  }}
                >
                  {folder
                    ? row.expanded
                      ? <ExpandMore sx={{ fontSize: "1.375rem" }} />
                      : <ChevronRight sx={{ fontSize: "1.375rem" }} />
                    : (
                      <StatusDot
                        status={row.session.status}
                        backgroundTasks={row.session.background_tasks}
                      />
                    )}
                </Box>
                {folder ? <FolderRowContent row={row} /> : (
                  <SessionRowContent
                    session={row.session}
                    current={current}
                  />
                )}
              </ReliableListItemButton>
            );
          })}
        </List>
        {!choosable && (
          <Typography role="status" color="text.secondary" sx={{ p: 2 }}>
            {needle ? "No matching Sessions" : "No other Sessions yet."}
          </Typography>
        )}
      </Box>
    </Stack>
  );
}

function FolderRowContent(
  { row }: { row: FolderRow },
): React.JSX.Element {
  const Icon = row.expanded ? FolderOpenOutlined : FolderOutlined;
  return (
    <Stack
      direction="row"
      spacing={0.75}
      alignItems="center"
      sx={{ minWidth: 0, flex: 1 }}
    >
      <Icon sx={{ flexShrink: 0, color: "text.secondary" }} />
      <Typography variant="body2" noWrap sx={{ minWidth: 0, fontWeight: 600 }}>
        {row.folder.name}
      </Typography>
      {row.folder.project && (
        <Chip
          size="small"
          variant="outlined"
          icon={<LabelOutlined sx={{ fontSize: "0.9rem !important" }} />}
          label={row.folder.project}
          sx={{
            height: "1.375rem",
            maxWidth: "8rem",
            fontSize: "0.75rem",
            flexShrink: 1,
            minWidth: 0,
            "& .MuiChip-label": {
              px: "0.5rem",
              overflow: "hidden",
              textOverflow: "ellipsis",
            },
          }}
        />
      )}
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}
      >
        {row.sessionCount}
      </Typography>
    </Stack>
  );
}

function SessionRowContent(
  { session, current }: { session: SessionMeta; current: boolean },
): React.JSX.Element {
  return (
    <Stack
      sx={{
        minWidth: 0,
        flex: 1,
        color: current ? "text.secondary" : undefined,
      }}
    >
      <Stack
        direction="row"
        spacing={0.75}
        alignItems="center"
        sx={{ minWidth: 0 }}
      >
        <ProviderIcon
          provider={session.provider}
          providerVersion={session.provider_version}
          providerDigest={session.provider_generation_digest}
          sx={{ flexShrink: 0 }}
        />
        <Typography variant="body2" noWrap sx={{ minWidth: 0 }}>
          {session.title || "Session"}
        </Typography>
        {current && (
          <Chip
            size="small"
            label="Current"
            sx={{
              height: "1.25rem",
              fontSize: "0.6875rem",
              flexShrink: 0,
              "& .MuiChip-label": { px: "0.5rem" },
            }}
          />
        )}
      </Stack>
      <Stack
        direction="row"
        spacing={0.75}
        alignItems="center"
        sx={{ minWidth: 0, maxWidth: "100%", color: "text.secondary" }}
      >
        <Typography variant="caption" noWrap sx={{ minWidth: 0 }}>
          {sessionListProjectLabel(session)}
        </Typography>
        <SessionMachineBadge session={session} compact />
      </Stack>
    </Stack>
  );
}
