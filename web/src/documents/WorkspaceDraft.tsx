import {
  Box,
  Button,
  IconButton,
  List,
  ListItemButton,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import {
  DeleteOutline,
  DescriptionOutlined,
  DragIndicator,
  MoreVert,
  RestoreFromTrashOutlined,
} from "@mui/icons-material";
import { type MutableRefObject, useState } from "react";
import { flushSync } from "react-dom";
import type { SxProps, Theme } from "@mui/material";
import type { Sortable } from "../useSortable";
import { ReliableListItemButton } from "../ReliableListItemButton";
import { FolderNameShell, FolderPickerShell } from "../SessionFolderUi";
import { useConfirmEnter } from "../Kbd";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import { ConfirmSheet, Sheet } from "../Sheet";
import { useStoreSelector } from "../store";
import {
  DraftDestinationModal,
  DraftDestinationSheet,
} from "../DraftDestinationPicker";
import {
  DraftEditor,
  type DraftFlush,
  type DraftMobileChrome,
} from "./DraftEditor";
import { DRAFT_DRAG_TYPE, type DraftChange, type DraftMetadata } from "./model";
import { draftRepository, useDraftLibrary } from "./store";
import { documentNotice } from "./DocumentNotifications";
import { copyDraftToSession } from "./transfer";

export type WorkspaceDraftAction = {
  draft: DraftMetadata;
  action: "menu" | "rename" | "move" | "trash" | "copy";
};

export function WorkspaceDraftRow(
  { draft, selected, sortable, sx, onPick, onAction, desktop }: {
    draft: DraftMetadata;
    selected: boolean;
    sortable: Sortable;
    sx: SxProps<Theme>;
    onPick: () => void;
    onAction: (action: WorkspaceDraftAction) => void;
    desktop: boolean;
  },
): React.JSX.Element {
  const key = `draft:${draft.id}`;
  return (
    <ReliableListItemButton
      data-desktop-item={key}
      data-workspace-kind="draft"
      data-desktop-current={selected ? "true" : undefined}
      selected={selected}
      ref={sortable.registerItem(key)}
      style={sortable.itemStyle(key)}
      onActivate={onPick}
      sx={sx}
      draggable={desktop}
      onDragStart={(event) => {
        event.dataTransfer.setData(DRAFT_DRAG_TYPE, draft.id);
        event.dataTransfer.effectAllowed = "copyMove";
      }}
    >
      <IconButton
        className="cowboy-session-grip"
        aria-label={`Drag ${draft.title}`}
        {...sortable.handleProps(key)}
        sx={{
          minWidth: "2.75rem",
          minHeight: "2.75rem",
          fontSize: "inherit",
          color: "text.disabled",
        }}
      >
        <DragIndicator sx={{ fontSize: "1.125rem" }} />
      </IconButton>
      <DescriptionOutlined
        sx={{
          fontSize: "1.25rem",
          mx: 1,
          color: "text.secondary",
          flexShrink: 0,
        }}
      />
      <ListItemText
        primary={draft.title}
        secondary="Draft"
        slotProps={{
          primary: { noWrap: true },
          secondary: { fontSize: "0.75rem" },
        }}
      />
      <IconButton
        className="cowboy-session-actions"
        aria-label={`Actions for ${draft.title}`}
        onClick={(event) => {
          event.stopPropagation();
          onAction({ draft, action: "menu" });
        }}
        sx={{ minWidth: "2.75rem", minHeight: "2.75rem" }}
      >
        <MoreVert sx={{ fontSize: "1.125rem" }} />
      </IconButton>
    </ReliableListItemButton>
  );
}

export function WorkspaceDraftPane(
  { id, beforeLeave, onAction, mobileNavigation }: {
    id: string;
    beforeLeave: MutableRefObject<DraftFlush>;
    onAction: (action: WorkspaceDraftAction) => void;
    /** Mobile only: the page owns its navigation (no session bottom nav). */
    mobileNavigation?:
      | Pick<DraftMobileChrome, "onOpenSessions">
      | undefined;
  },
): React.JSX.Element {
  const library = useDraftLibrary();
  const draft = library.entries.find((entry) => entry.id === id);
  return (
    <Stack
      data-workspace-document={id}
      sx={{ flex: 1, height: "100%", minHeight: 0, minWidth: 0 }}
    >
      <DraftEditor
        key={id}
        id={id}
        beforeLeave={beforeLeave}
        onCopyToSession={() => draft && onAction({ draft, action: "copy" })}
        mobileChrome={mobileNavigation && {
          ...mobileNavigation,
          onMenu: () => draft && onAction({ draft, action: "menu" }),
        }}
      />
    </Stack>
  );
}

export function WorkspaceDraftActions(
  { request, onClose, beforeLeave, activeId, onRemoved }: {
    request: WorkspaceDraftAction;
    onClose: () => void;
    beforeLeave: MutableRefObject<DraftFlush>;
    activeId: string | null;
    onRemoved: () => void;
  },
): React.JSX.Element {
  const folders = useStoreSelector((snapshot) => snapshot.sessionFolders);
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const [action, setAction] = useState(request.action);
  const [busy, setBusy] = useState(false);
  const order = useStoreSelector((snapshot) => snapshot.workspaceOrder);
  const desktop = useSurfaceProfile().kind === "desktop";
  const run = (operation: () => Promise<void>): void => {
    if (busy) return;
    setBusy(true);
    void (async () => {
      if (activeId === request.draft.id) await beforeLeave.current();
      await operation();
      onClose();
    })().catch((error: Error) => documentNotice(error.message)).finally(() =>
      setBusy(false)
    );
  };
  const change = (change: DraftChange) =>
    run(async () => {
      const owner = draftRepository().document(request.draft.id);
      await owner.hydrate();
      await owner.change(change);
      if (change.type === "trash") {
        if (activeId === request.draft.id) onRemoved();
        documentNotice(
          "Moved to Trash",
          () => owner.change({ type: "restore" }),
        );
      }
    });
  useConfirmEnter(
    desktop && action === "trash",
    () => change({ type: "trash" }),
  );
  if (action === "menu") {
    return (
      <Sheet open onClose={onClose} title={request.draft.title}>
        <List>
          {(["rename", "move", "copy", "trash"] as const).map((item) => (
            <ListItemButton
              key={item}
              onClick={() => flushSync(() => setAction(item))}
            >
              <ListItemText
                primary={{
                  rename: "Rename",
                  move: "Move to…",
                  copy: "Add to Session…",
                  trash: "Move to Trash",
                }[item]}
              />
            </ListItemButton>
          ))}
        </List>
      </Sheet>
    );
  }
  if (action === "move") {
    return (
      <FolderPickerShell
        title={`Move ${request.draft.title}`}
        rootLabel="Top level"
        value={folders}
        current={request.draft.parent_id}
        exclude={null}
        onClose={onClose}
        onPick={(parent_id) => change({ type: "move", parent_id })}
      />
    );
  }
  if (action === "rename") {
    return (
      <FolderNameShell
        title="Rename draft"
        initial={request.draft.title}
        confirmLabel="Rename"
        normalize={(name) => {
          const value = name.trim();
          return value && [...value].length <= 160 && !/\p{Cc}/u.test(value)
            ? value
            : null;
        }}
        onClose={onClose}
        onConfirm={(title) => change({ type: "rename", title })}
      />
    );
  }
  if (action === "copy") {
    const Picker = desktop ? DraftDestinationModal : DraftDestinationSheet;
    return (
      <Picker
        title="Add draft to Session"
        sourceId=""
        initialFolder={request.draft.parent_id}
        busy={busy}
        sessions={sessions}
        folders={folders}
        order={order}
        onClose={onClose}
        onPick={(session) =>
          run(() =>
            copyDraftToSession(request.draft.id, session.id, session.title)
          )}
      />
    );
  }
  return (
    <ConfirmSheet
      open
      onClose={busy ? () => {} : onClose}
      title={`Move “${request.draft.title}” to Trash?`}
      actions={
        <>
          <Button onClick={onClose} disabled={busy}>Cancel</Button>
          <Button
            color="error"
            variant="contained"
            disabled={busy}
            onClick={() => change({ type: "trash" })}
          >
            Move to Trash
          </Button>
        </>
      }
    >
      <Typography>The document can be restored from Trash.</Typography>
    </ConfirmSheet>
  );
}

export function WorkspaceDraftTrash(): React.JSX.Element {
  const library = useDraftLibrary();
  const [open, setOpen] = useState(false);
  const trash = library.entries.filter((entry) =>
    entry.kind === "document" && entry.deleted
  );
  if (!trash.length && !open) return <></>;
  return (
    <>
      <Button
        startIcon={<DeleteOutline />}
        onClick={() => setOpen(true)}
        sx={{ textTransform: "none", minHeight: "2.75rem" }}
      >
        Trash{trash.length ? ` · ${trash.length}` : ""}
      </Button>
      <Sheet open={open} onClose={() => setOpen(false)} title="Draft Trash">
        <List>
          {trash.map((draft) => (
            <ListItemButton
              key={draft.id}
              onClick={() => {
                void draftRepository().document(draft.id).hydrate().then(() =>
                  draftRepository().document(draft.id).change({
                    type: "restore",
                  })
                ).catch((error: Error) => documentNotice(error.message));
              }}
            >
              <RestoreFromTrashOutlined sx={{ mr: 1 }} />
              <ListItemText primary={draft.title} secondary="Restore" />
            </ListItemButton>
          ))}
        </List>
        {trash.length === 0 && <Box sx={{ p: 2 }}>Trash is empty.</Box>}
      </Sheet>
    </>
  );
}
