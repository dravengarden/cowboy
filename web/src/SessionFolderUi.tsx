// Sessions-sidebar folder chrome shared by the Mobile drawer and the Desktop
// rail (docs/sessions-folders.md): the per-device collapsed set, the folder
// name prompt, the Move-to / Bind-project pickers and the delete confirm.
// The tree itself is rendered by SessionList; these are its transient layers.

import {
  Button,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import {
  Check as CheckIcon,
  FolderOutlined,
  LabelOutlined,
  ViewListOutlined,
} from "@mui/icons-material";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import { isImeKeyEvent } from "./imeKey";
import { Kbd, useConfirmEnter } from "./Kbd";
import { useNavbarAtBottom } from "./navbarSettings";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import {
  childFolders,
  folderIsWithin,
  normalizeSessionFolderName,
  type SessionFolder,
  type SessionFoldersValue,
} from "./sessionFolders";
import { Sheet } from "./Sheet";
import { useSurfaceProfile } from "./surface/SurfaceProfile";

const COLLAPSED_KEY = "cowboy:session-folders:collapsed";

function readCollapsed(): ReadonlySet<string> {
  try {
    const parsed: unknown = JSON.parse(
      globalThis.localStorage.getItem(COLLAPSED_KEY) ?? "[]",
    );
    return new Set(
      Array.isArray(parsed)
        ? parsed.filter((id): id is string => typeof id === "string")
        : [],
    );
  } catch {
    return new Set();
  }
}

/** Collapsed folder ids for this device. Presentation, never synced: Obsidian
 *  keeps its folds in localStorage for the same reason. */
export function useCollapsedSessionFolders(): readonly [
  ReadonlySet<string>,
  (update: (previous: ReadonlySet<string>) => ReadonlySet<string>) => void,
] {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(
    readCollapsed,
  );
  useEffect(() => {
    try {
      globalThis.localStorage.setItem(
        COLLAPSED_KEY,
        JSON.stringify([...collapsed]),
      );
    } catch {
      // Private/transient WebViews may refuse storage; folds then last a session.
    }
  }, [collapsed]);
  const update = useCallback(
    (fn: (previous: ReadonlySet<string>) => ReadonlySet<string>): void => {
      setCollapsed((previous) => {
        const next = fn(previous);
        return next === previous ? previous : next;
      });
    },
    [],
  );
  return [collapsed, update] as const;
}

export function withFoldersCollapsed(
  collapsed: ReadonlySet<string>,
  ids: readonly string[],
  value: boolean,
): ReadonlySet<string> {
  if (ids.every((id) => collapsed.has(id) === value)) return collapsed;
  const next = new Set(collapsed);
  for (const id of ids) {
    if (value) next.add(id);
    else next.delete(id);
  }
  return next;
}

/** Folders in tree order (depth-first by position) with their depth. */
export function flattenFolders(
  value: SessionFoldersValue,
  exclude: string | null = null,
): { folder: SessionFolder; depth: number }[] {
  const out: { folder: SessionFolder; depth: number }[] = [];
  const seen = new Set<string>();
  const walk = (parent: string | null, depth: number): void => {
    for (const folder of childFolders(value, parent)) {
      if (seen.has(folder.id)) continue;
      seen.add(folder.id);
      if (exclude && folderIsWithin(value, folder.id, exclude)) continue;
      out.push({ folder, depth });
      walk(folder.id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

/** Vim/arrow motion among the picker rows; Enter activates the focused row. */
function pickerKeyDown(event: React.KeyboardEvent<HTMLElement>): void {
  if (isImeKeyEvent(event.nativeEvent)) return;
  const delta = event.key === "ArrowDown" || event.key === "j"
    ? 1
    : event.key === "ArrowUp" || event.key === "k"
    ? -1
    : 0;
  if (delta === 0 || event.metaKey || event.ctrlKey || event.altKey) return;
  const rows = [
    ...event.currentTarget.querySelectorAll<HTMLElement>("[data-folder-pick]"),
  ].filter((row) =>
    !row.hasAttribute("aria-disabled") ||
    row.getAttribute("aria-disabled") !== "true"
  );
  if (rows.length === 0) return;
  event.preventDefault();
  const current = rows.indexOf(document.activeElement as HTMLElement);
  const next = current < 0
    ? (delta > 0 ? 0 : rows.length - 1)
    : Math.max(0, Math.min(rows.length - 1, current + delta));
  rows[next]?.focus({ preventScroll: true });
  rows[next]?.scrollIntoView({ block: "nearest" });
}

export function FolderNameShell({
  title,
  initial,
  confirmLabel,
  helperText,
  extra,
  onClose,
  onConfirm,
}: {
  title: string;
  initial: string;
  confirmLabel: string;
  helperText?: string;
  /** Secondary action rendered beside Cancel (Mobile "Organize by project"). */
  extra?: ReactNode;
  onClose: () => void;
  onConfirm: (name: string) => void;
}): React.JSX.Element {
  const [value, setValue] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);
  const navbarAtBottom = useNavbarAtBottom();
  const desktop = useSurfaceProfile().kind === "desktop";
  useLayoutEffect(() => {
    // Mounted with flushSync inside the opening tap so iOS raises the keyboard
    // for the real input (the same contract as RenameSessionShell).
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);
  const normalized = normalizeSessionFolderName(value);
  const canSave = normalized !== null && normalized !== initial;
  const submit = (): void => {
    if (normalized !== null && canSave) onConfirm(normalized);
  };
  useConfirmEnter(desktop, submit, { suppressBareEnter: false });
  return (
    <Sheet
      forceSheet={navbarAtBottom}
      open
      onClose={onClose}
      title={title}
      mobileDismiss="none"
      actions={
        <>
          {extra}
          <Button onClick={onClose} color="inherit">
            Cancel
            <Kbd keys="Esc" />
          </Button>
          <Button
            onClick={submit}
            onKeyDown={(e): void => {
              if (
                desktop && e.key === "Enter" && !e.metaKey && !e.ctrlKey &&
                !isImeKeyEvent(e.nativeEvent)
              ) e.preventDefault();
            }}
            variant="contained"
            disabled={!canSave}
          >
            {confirmLabel}
            <Kbd
              keys={`${MOD_LABEL}${ENTER_LABEL}`}
              availability={canSave ? "available" : "inactive"}
            />
          </Button>
        </>
      }
    >
      <TextField
        fullWidth
        inputRef={inputRef}
        label="Folder name"
        value={value}
        onChange={(e): void => setValue(e.target.value)}
        onKeyDown={(e): void => {
          if (
            e.key === "Enter" && !e.shiftKey && !isImeKeyEvent(e.nativeEvent)
          ) {
            e.preventDefault();
            if (!desktop) submit();
          }
        }}
        sx={{ mt: 1 }}
        helperText={helperText}
      />
    </Sheet>
  );
}

export function FolderPickerShell({
  title,
  value,
  current,
  exclude = null,
  onPick,
  onNewFolder,
  onClose,
}: {
  title: string;
  value: SessionFoldersValue;
  /** Folder the item is in today (`null` = top level); shown checked. */
  current: string | null;
  /** A folder being moved: it and its subtree cannot be a destination. */
  exclude?: string | null;
  onPick: (folder: string | null) => void;
  onNewFolder?: (() => void) | undefined;
  onClose: () => void;
}): React.JSX.Element {
  const navbarAtBottom = useNavbarAtBottom();
  const rows = flattenFolders(value, exclude);
  const listRef = useRef<HTMLUListElement>(null);
  useLayoutEffect(() => {
    // Start on the current location so Enter is a no-op and j/k moves from
    // where the item already lives.
    const key = current ?? "";
    const row = listRef.current?.querySelector<HTMLElement>(
      `[data-folder-pick="${CSS.escape(key)}"]`,
    );
    (row ?? listRef.current?.querySelector<HTMLElement>("[data-folder-pick]"))
      ?.focus({ preventScroll: true });
  }, [current]);
  return (
    <Sheet
      forceSheet={navbarAtBottom}
      open
      onClose={onClose}
      title={title}
      mobileDismiss="none"
      actions={
        <>
          {onNewFolder && (
            <Button onClick={onNewFolder} color="inherit">
              New folder…
            </Button>
          )}
          <Button onClick={onClose} color="inherit">
            Cancel
            <Kbd keys="Esc" />
          </Button>
        </>
      }
    >
      <List
        dense
        ref={listRef}
        onKeyDown={pickerKeyDown}
        sx={{ maxHeight: "min(60vh, 480px)", overflowY: "auto", mx: -1 }}
      >
        <ListItemButton
          data-folder-pick=""
          selected={current === null}
          onClick={(): void => onPick(null)}
        >
          <ListItemIcon sx={{ minWidth: 36 }}>
            {current === null ? <CheckIcon /> : <ViewListOutlined />}
          </ListItemIcon>
          <ListItemText primary="Top level" />
        </ListItemButton>
        {rows.map(({ folder, depth }) => {
          const isCurrent = folder.id === current;
          return (
            <ListItemButton
              key={folder.id}
              data-folder-pick={folder.id}
              selected={isCurrent}
              onClick={(): void => onPick(folder.id)}
              sx={{ pl: 2 + depth * 2.5 }}
            >
              <ListItemIcon sx={{ minWidth: 36 }}>
                {isCurrent ? <CheckIcon /> : <FolderOutlined />}
              </ListItemIcon>
              <ListItemText
                primary={folder.name}
                secondary={folder.project ?? undefined}
                slotProps={{
                  primary: { noWrap: true },
                  secondary: { noWrap: true, variant: "caption" },
                }}
              />
            </ListItemButton>
          );
        })}
        {rows.length === 0 && (
          <Typography
            variant="body2"
            color="text.secondary"
            sx={{ px: 2, py: 1.5 }}
          >
            No folders yet.
          </Typography>
        )}
      </List>
    </Sheet>
  );
}

export function ProjectPickerShell({
  folder,
  labels,
  onPick,
  onClose,
}: {
  folder: SessionFolder;
  /** Project labels seen among sessions (bound or not), first-seen order. */
  labels: readonly string[];
  onPick: (project: string | null) => void;
  onClose: () => void;
}): React.JSX.Element {
  const navbarAtBottom = useNavbarAtBottom();
  const listRef = useRef<HTMLUListElement>(null);
  const options = folder.project && !labels.includes(folder.project)
    ? [folder.project, ...labels]
    : labels;
  useLayoutEffect(() => {
    const row = listRef.current?.querySelector<HTMLElement>(
      `[data-folder-pick="${CSS.escape(folder.project ?? "")}"]`,
    );
    (row ?? listRef.current?.querySelector<HTMLElement>("[data-folder-pick]"))
      ?.focus({ preventScroll: true });
  }, [folder.project]);
  return (
    <Sheet
      forceSheet={navbarAtBottom}
      open
      onClose={onClose}
      title={`Bind ${folder.name} to a project`}
      mobileDismiss="none"
      actions={
        <Button onClick={onClose} color="inherit">
          Cancel
          <Kbd keys="Esc" />
        </Button>
      }
    >
      <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
        Sessions of the bound project appear in this folder automatically,
        unless they were moved somewhere else by hand.
      </Typography>
      <List
        dense
        ref={listRef}
        onKeyDown={pickerKeyDown}
        sx={{ maxHeight: "min(60vh, 480px)", overflowY: "auto", mx: -1 }}
      >
        <ListItemButton
          data-folder-pick=""
          selected={folder.project === null}
          onClick={(): void => onPick(null)}
        >
          <ListItemIcon sx={{ minWidth: 36 }}>
            {folder.project === null ? <CheckIcon /> : <LabelOutlined />}
          </ListItemIcon>
          <ListItemText primary="No project" />
        </ListItemButton>
        {options.map((label) => {
          const isCurrent = label === folder.project;
          return (
            <ListItemButton
              key={label}
              data-folder-pick={label}
              selected={isCurrent}
              onClick={(): void => onPick(label)}
            >
              <ListItemIcon sx={{ minWidth: 36 }}>
                {isCurrent ? <CheckIcon /> : <LabelOutlined />}
              </ListItemIcon>
              <ListItemText
                primary={label}
                slotProps={{ primary: { noWrap: true } }}
              />
            </ListItemButton>
          );
        })}
        {options.length === 0 && (
          <Typography
            variant="body2"
            color="text.secondary"
            sx={{ px: 2, py: 1.5 }}
          >
            No project labels among the current sessions.
          </Typography>
        )}
      </List>
    </Sheet>
  );
}

export function DeleteFolderShell({
  folder,
  sessionCount,
  destination,
  onClose,
  onConfirm,
}: {
  folder: SessionFolder;
  sessionCount: number;
  /** Name of the folder that receives the contents, or `null` = top level. */
  destination: string | null;
  onClose: () => void;
  onConfirm: () => void;
}): React.JSX.Element {
  const navbarAtBottom = useNavbarAtBottom();
  useConfirmEnter(true, onConfirm);
  const target = destination ? `"${destination}"` : "the top level";
  return (
    <Sheet
      forceSheet={navbarAtBottom}
      open
      onClose={onClose}
      title={`Delete folder "${folder.name}"?`}
      mobileDismiss="none"
      actions={
        <>
          <Button onClick={onClose} color="inherit">
            Cancel
            <Kbd keys="Esc" />
          </Button>
          <Button onClick={onConfirm} color="error" variant="contained">
            Delete folder
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </Button>
        </>
      }
    >
      <Stack spacing={1}>
        <Typography variant="body2" color="text.secondary">
          {sessionCount === 0
            ? `Nothing is deleted but the folder; its subfolders move to ${target}.`
            : `No session is deleted. Its ${sessionCount} ${
              sessionCount === 1 ? "session" : "sessions"
            } and any subfolders move to ${target}.`}
        </Typography>
        {folder.project && (
          <Typography variant="caption" color="text.secondary">
            Sessions of "{folder.project}" stop filing themselves here.
          </Typography>
        )}
      </Stack>
    </Sheet>
  );
}
