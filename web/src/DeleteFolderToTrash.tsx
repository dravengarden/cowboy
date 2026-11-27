import { ScrollEdges } from "@cowboy/app-shell";
// Delete a folder that may hold sessions or drafts (docs/sessions-folders.md#trash).
// The folder, its subfolders and every checked item go to Trash together and
// restore together. Unchecking an item keeps it: it moves to the destination
// first. "Move folder only to Trash" moves direct subfolders, sessions and
// drafts up to the destination first, then moves the emptied folder to Trash.

import { Box, Button, Checkbox, Stack, Typography } from "@mui/material";
import { ChatBubbleOutline, DeleteOutline, DescriptionOutlined } from "@mui/icons-material";
import { useMemo, useState } from "react";
import { draftRepository } from "./documents/store";
import { documentNotice } from "./documents/DocumentNotifications";
import { recordUndo } from "./undoActions";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { InAppRoot } from "./SessionFolderUi";
import { ConfirmSheet } from "./Sheet";
import type { SessionFolder, SessionFoldersValue } from "./sessionFolders";
import type { FolderDeletePlan } from "./trashModel";
import type { SessionMeta } from "./protocol";
import type { DraftMetadata } from "./documents/model";
import { trashFolderKeeping } from "./store";
import { trashFolderOnlyOnController } from "./documents/sessionDraftTrash";
import { desktopSize } from "./surface/desktopSize";

interface PlanItem {
  readonly key: string;
  readonly id: string;
  readonly kind: "session" | "draft";
  readonly title: string;
  readonly path: string;
  readonly working: boolean;
}

function count(value: number, noun: string, plural = `${noun}s`): string {
  return `${String(value)} ${value === 1 ? noun : plural}`;
}

function planItems(plan: FolderDeletePlan): PlanItem[] {
  return [
    ...plan.sessions.map((entry): PlanItem => ({
      key: `session:${entry.session.id}`,
      id: entry.session.id,
      kind: "session",
      title: entry.session.title || "Session",
      path: entry.path,
      working: entry.working,
    })),
    ...plan.drafts.map((entry): PlanItem => ({
      key: `draft:${entry.draft.id}`,
      id: entry.draft.id,
      kind: "draft",
      title: entry.draft.title || "Draft",
      path: entry.path,
      working: false,
    })),
  ];
}

/** The primary button names exactly what happens. */
export function deleteFolderActionLabel(total: number, kept: number): string {
  if (total === 0 || kept === 0) return "Move to Trash";
  if (kept === total) return `Keep ${String(kept)}, trash folder`;
  return `Trash ${String(total - kept)}, keep ${String(kept)}`;
}

function FolderDeleteItemList({
  items, keep, busy, destinationName, onToggle,
}: {
  items: readonly PlanItem[];
  keep: ReadonlySet<string>;
  busy: boolean;
  destinationName: string;
  onToggle: (key: string) => void;
}): React.JSX.Element {
  return (
    <Box
      component="ul"
      aria-label="Items in this folder"
      sx={{ m: 0, p: 0, maxHeight: "min(40vh, 320px)", overflowY: "auto", border: 1, borderColor: "divider", borderRadius: 1 }}
    >
      {items.map((item) => {
        const keeping = keep.has(item.key) || item.working;
        return (
          <Box
            key={item.key}
            component="li"
            data-delete-folder-item={item.key}
            sx={{ listStyle: "none", borderBottom: 1, borderColor: "divider", "&:last-child": { borderBottom: 0 } }}
          >
            <Box
              component="label"
              sx={{ display: "flex", alignItems: "center", gap: 1, pr: 1, minHeight: desktopSize(44), cursor: item.working ? "default" : "pointer" }}
            >
              <Checkbox
                size="small"
                checked={!keeping}
                disabled={busy || item.working}
                onChange={(): void => onToggle(item.key)}
                slotProps={{ input: { "aria-label": `Delete ${item.title}` } }}
                sx={{ flexShrink: 0 }}
              />
              {item.kind === "draft"
                ? <DescriptionOutlined sx={{ fontSize: "1.1rem", color: "text.secondary", flexShrink: 0 }} />
                : <ChatBubbleOutline sx={{ fontSize: "1.1rem", color: "text.secondary", flexShrink: 0 }} />}
              <Box sx={{ minWidth: 0, flex: 1 }}>
                <Typography variant="body2" noWrap>{item.title}</Typography>
                <Typography variant="caption" noWrap component="div" color={item.working ? "warning.main" : "text.secondary"}>
                  {item.working ? `Working, so it is kept in ${destinationName}` : keeping ? `Kept · moves to ${destinationName}` : item.path}
                </Typography>
              </Box>
            </Box>
          </Box>
        );
      })}
      <ScrollEdges />
    </Box>
  );
}

export function DeleteFolderToTrashShell(
  { folder, plan, destination, destinationName, retentionDays, onClose }: {
    folder: SessionFolder;
    plan: FolderDeletePlan;
    liveFolders?: SessionFoldersValue;
    sessions?: readonly SessionMeta[];
    drafts?: readonly DraftMetadata[];
    /** Where kept items go: the folder's parent, `null` = Global. */
    destination: string | null;
    destinationName: string;
    retentionDays: number;
    onClose: () => void;
  },
): React.JSX.Element {
  const items = useMemo(() => planItems(plan), [plan]);
  const working = useMemo(
    () => new Set(items.filter((item) => item.working).map((item) => item.key)),
    [items],
  );
  const [keep, setKeep] = useState<ReadonlySet<string>>(() => working);
  const [busy, setBusy] = useState(false);
  const kept = items.filter((item) => keep.has(item.key) || item.working).length;
  const confirm = (): void => {
    if (busy) return;
    setBusy(true);
    const keptItems = items.filter((item) => keep.has(item.key) || item.working);
    void (async (): Promise<void> => {
      for (const item of keptItems.filter((entry) => entry.kind === "draft")) {
        const owner = draftRepository().document(item.id);
        await owner.hydrate();
        await owner.change({ type: "move", parent_id: destination });
        await owner.whenSynced();
      }
      await trashFolderKeeping(
        folder.id,
        keptItems.filter((entry) => entry.kind === "session").map((entry) => entry.id),
        destination,
      );
    })().then(() => {
      onClose();
      recordUndo(
        kept ? `"${folder.name}" moved to Trash; ${String(kept)} kept in ${destinationName}` : `"${folder.name}" moved to Trash`,
        [{ kind: "restore_trash", folders: [folder.id] }],
      );
    }).catch((error: unknown) => {
      documentNotice(error instanceof Error ? error.message : "Could not move the folder to Trash.");
    }).finally(() => setBusy(false));
  };
  useConfirmEnter(!busy, confirm);
  const toggle = (key: string): void =>
    setKeep((previous) => {
      const next = new Set(previous);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const contents = [
    plan.sessions.length ? count(plan.sessions.length, "session") : null,
    plan.drafts.length ? count(plan.drafts.length, "Draft") : null,
    plan.subfolders ? count(plan.subfolders, "subfolder") : null,
  ].filter((part): part is string => part !== null);
  return (
    <InAppRoot>
      <ConfirmSheet
        portal={false}
        open
        onClose={onClose}
        title={`Delete "${folder.name}"?`}
        actions={
          <>
            <Button onClick={onClose} color="inherit" disabled={busy}>
              Cancel
              <Kbd keys="Esc" />
            </Button>
            <Button onClick={confirm} color="error" variant="contained" disabled={busy} startIcon={<DeleteOutline />}>
              {deleteFolderActionLabel(items.length, kept)}
              <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
            </Button>
          </>
        }
      >
        <Stack spacing={1}>
          <Typography variant="body2" color="text.secondary">
            {contents.length
              ? `Contains ${contents.join(", ")}. Checked items go to Trash with the folder and keep its structure. Everything can be restored for ${count(retentionDays, "day")}.`
              : `The empty folder goes to Trash for ${count(retentionDays, "day")}.`}
          </Typography>
          {items.length > 0 && (
            <>
              <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
                <Typography variant="body2" sx={{ fontWeight: 600 }}>
                  {kept === 0
                    ? "Delete everything inside?"
                    : `Keeping ${String(kept)} · ${kept === 1 ? "it moves" : "they move"} to ${destinationName}`}
                </Typography>
                <Stack direction="row" spacing={0.5} sx={{ flexShrink: 0 }}>
                  <Button size="small" disabled={busy} onClick={(): void => setKeep(working)}>
                    Delete all
                  </Button>
                  <Button size="small" disabled={busy} onClick={(): void => setKeep(new Set(items.map((item) => item.key)))}>
                    Keep all
                  </Button>
                </Stack>
              </Stack>
              {kept === 0 && (
                <Typography variant="caption" color="text.secondary">
                  Not all of them? Uncheck the sessions to keep before deleting.
                </Typography>
              )}
              <FolderDeleteItemList items={items} keep={keep} busy={busy} destinationName={destinationName} onToggle={toggle} />
            </>
          )}
          {contents.length > 0 && (
            <Button
              size="small"
              color="inherit"
              disabled={busy}
              onClick={(): void => {
                if (busy) return;
                setBusy(true);
                void trashFolderOnlyOnController(folder.id)
                  .then((delta) => {
                    onClose();
                    recordUndo(`"${folder.name}" moved to Trash; contents kept in ${destinationName}`, [{
                      kind: "restore_folder_only",
                      folderId: delta.folder_id,
                      receiptId: delta.receipt_id,
                    }]);
                  })
                  .catch((error: unknown) => {
                    documentNotice(error instanceof Error ? error.message : "Could not move the folder to Trash.");
                  })
                  .finally(() => setBusy(false));
              }}
              sx={{ alignSelf: "flex-start", textAlign: "left" }}
            >
              Move folder only to Trash — keep everything in {destinationName}
            </Button>
          )}
        </Stack>
      </ConfirmSheet>
    </InAppRoot>
  );
}
