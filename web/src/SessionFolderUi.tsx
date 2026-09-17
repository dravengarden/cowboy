// Sessions-sidebar folder chrome shared by the Mobile drawer and the Desktop
// rail (docs/sessions-folders.md): the per-device collapsed set, the folder
// name prompt, the Move-to / Bind-project pickers and the delete confirm.
// The tree itself is rendered by SessionList; these are its transient layers.
//
// Every sheet here is portalled to the host App renders beside the session
// Rename/Delete shells (`SESSION_FOLDER_SHEET_HOST`), never inline and never
// to <body>:
// - SessionList lives inside the Mobile drawer layer (a transformed,
//   overflow-hidden compositor layer stacked under the page peek), so an
//   inline `position: fixed` sheet is laid out against the drawer, clipped to
//   its width and painted beneath the page (physical iPhone, 2026-09-17).
// - A <body>-level compact sheet escapes the keyboard-resized app box: on a
//   physical iPad the name prompt sat behind the keyboard and dismissing it
//   left the page lifted a second keyboard height (2026-09-17). Only cover
//   sheets, which pin to --vv-height/--vv-offset, may live on <body>.
// The Rename shell's mount point is the one placement proven on devices, so
// the folder sheets share its exact ancestry.

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
import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
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
import { useSheetKeyboardDiagnostics } from "./sheetKeyboardDiagnostics";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { useDialogFocus, useDialogInputFocus } from "./useDialogInputFocus";

/** Attribute of the mount point App renders beside the Rename shell. */
export const SESSION_FOLDER_SHEET_HOST = "data-session-folder-sheets";

/** Render in App's root beside the session shells. A React portal commits in
 *  the same pass, so a prompt's in-tap focus (iOS keyboard) still holds. */
function InAppRoot({ children }: { children: ReactNode }): ReactNode {
  const host = globalThis.document?.querySelector(
    `[${SESSION_FOLDER_SHEET_HOST}]`,
  );
  return host ? createPortal(children, host) : children;
}

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
  /** Secondary action rendered under the field (Mobile "Organize by
   *  project"); the action row keeps exactly Cancel + confirm. */
  extra?: ReactNode;
  onClose: () => void;
  onConfirm: (name: string) => void;
}): React.JSX.Element {
  const [value, setValue] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);
  const navbarAtBottom = useNavbarAtBottom();
  const desktop = useSurfaceProfile().kind === "desktop";
  // Mounted with flushSync inside the opening tap so iOS raises the keyboard
  // for the real input (the same contract as RenameSessionShell).
  useDialogInputFocus(inputRef, desktop);
  useSheetKeyboardDiagnostics(
    "session-folder-name",
    !desktop,
    () => inputRef.current?.closest("[role='dialog']") ?? null,
  );
  const normalized = normalizeSessionFolderName(value);
  const canSave = normalized !== null && normalized !== initial;
  const submit = (): void => {
    if (normalized !== null && canSave) onConfirm(normalized);
  };
  useConfirmEnter(desktop, submit, { suppressBareEnter: false });
  return (
    <InAppRoot>
      <Sheet
        forceSheet={navbarAtBottom}
        open
        onClose={onClose}
        title={title}
        mobileDismiss="none"
        actions={
          <>
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
          // A folder is not a contact: without this iOS reads "name" and
          // raises the AutoFill Contact bar, which also regrows the keyboard
          // after it has been presented (physical iPad, 2026-09-17).
          autoComplete="off"
          name="cowboy-session-folder"
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
        {extra}
      </Sheet>
    </InAppRoot>
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
  const desktop = useSurfaceProfile().kind === "desktop";
  const rows = flattenFolders(value, exclude);
  const listRef = useRef<HTMLUListElement>(null);
  // Start on the current location so Enter is a no-op and j/k moves from
  // where the item already lives.
  useDialogFocus(
    () =>
      listRef.current?.querySelector<HTMLElement>(
        `[data-folder-pick="${CSS.escape(current ?? "")}"]`,
      ) ?? listRef.current?.querySelector<HTMLElement>("[data-folder-pick]"),
    desktop,
  );
  return (
    <InAppRoot>
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
    </InAppRoot>
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
  const desktop = useSurfaceProfile().kind === "desktop";
  const listRef = useRef<HTMLUListElement>(null);
  const options = folder.project && !labels.includes(folder.project)
    ? [folder.project, ...labels]
    : labels;
  useDialogFocus(
    () =>
      listRef.current?.querySelector<HTMLElement>(
        `[data-folder-pick="${CSS.escape(folder.project ?? "")}"]`,
      ) ?? listRef.current?.querySelector<HTMLElement>("[data-folder-pick]"),
    desktop,
  );
  return (
    <InAppRoot>
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
    </InAppRoot>
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
    <InAppRoot>
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
    </InAppRoot>
  );
}
