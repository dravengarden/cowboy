import { Suspense } from "react";
import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import type { QueuedMessage } from "../store";
import { DESKTOP_SHORTCUTS } from "./commands/workspaceShortcuts";
import { DesktopRegionShortcut } from "./DesktopRegionShortcut";
import { firstLinePreview } from "./promptBuffer";

/** Right-side shortcut keycap + reorder indicator in Desktop Queue/Draft headers. */
export function DesktopPendingHeaderTrailing({
  kind,
  visuallyCollapsed,
  reordering,
}: {
  kind: "queued" | "draft";
  visuallyCollapsed: boolean;
  reordering: boolean;
}): React.JSX.Element {
  return (
    <>
      <Stack
        direction="row"
        spacing={0.5}
        alignItems="center"
        sx={{
          display: "none",
          "@container pendingPanel (min-width: 24rem)": { display: "flex" },
        }}
      >
        {kind === "queued" && (
          <Suspense fallback={null}>
            <DesktopRegionShortcut
              shortcut={DESKTOP_SHORTCUTS.focusQueue}
              title={visuallyCollapsed
                ? "Open and focus queue"
                : "Close queue when focused"}
              singleKeycap={DESKTOP_SHORTCUTS.focusQueue}
            />
          </Suspense>
        )}
        {kind === "draft" && (
          <Suspense fallback={null}>
            <DesktopRegionShortcut
              shortcut={DESKTOP_SHORTCUTS.focusDrafts}
              title="Focus drafts"
              singleKeycap={DESKTOP_SHORTCUTS.focusDrafts}
            />
          </Suspense>
        )}
      </Stack>
      {reordering && (
        <Typography
          variant="caption"
          color="primary.main"
          sx={{
            ml: 0.75,
            minWidth: 0,
            fontWeight: 700,
            letterSpacing: "0.04em",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          REORDER · J/K MOVE · ESC DONE
        </Typography>
      )}
    </>
  );
}

/** Glanceable 1-line preview + edit/hold badges inside Desktop Queue/Draft headers. */
export function DesktopPendingHeaderSummary({
  kind,
  items,
  reordering,
  activeEditId,
  editingHeld,
}: {
  kind: "queued" | "draft";
  items: readonly QueuedMessage[];
  reordering: boolean;
  activeEditId?: string | null | undefined;
  editingHeld?: boolean | undefined;
}): React.JSX.Element {
  return (
    <>
      {items.length > 0 && !reordering && items[0] && (
        <Typography
          variant="caption"
          color="text.secondary"
          data-desktop-pending-preview
          sx={{
            ml: 0.75,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            opacity: 0.85,
          }}
        >
          · {firstLinePreview(items[0].text)}
        </Typography>
      )}
      {activeEditId != null && (
        <Typography
          variant="caption"
          color="primary.main"
          sx={{ ml: 0.75, fontWeight: 700, flexShrink: 0 }}
        >
          Editing
        </Typography>
      )}
      {kind === "queued" && editingHeld && (
        <Box
          sx={{
            display: "inline-flex",
            alignItems: "center",
            ml: 0.75,
            px: 0.625,
            py: 0.125,
            borderRadius: 1,
            bgcolor: (t) => alpha(t.palette.warning.main, 0.16),
            color: "warning.main",
            flexShrink: 0,
          }}
        >
          <Typography variant="caption" sx={{ fontWeight: 700, lineHeight: 1.2 }}>
            Auto-drain held
          </Typography>
        </Box>
      )}
    </>
  );
}

/** Empty-state row shown when Desktop `␣D` opens Session drafts at count 0. */
export function DesktopEmptyDraftsRow({
  onNewDraft,
}: {
  onNewDraft?: (() => void) | undefined;
}): React.JSX.Element {
  return (
    <Stack
      direction="row"
      alignItems="center"
      justifyContent="space-between"
      spacing={1}
      data-desktop-empty-drafts-row
      sx={{ px: 1, py: 0.75 }}
    >
      <Typography variant="caption" color="text.secondary">
        No drafts yet · Press N to start a draft · Esc to return
      </Typography>
      {onNewDraft && (
        <Button
          size="small"
          variant="outlined"
          onClick={onNewDraft}
          sx={{ textTransform: "none", py: 0.25, minHeight: 26 }}
        >
          New draft
        </Button>
      )}
    </Stack>
  );
}

/** Handle `DESKTOP_PENDING_ROW_EDIT_EVENT` bubbling up from a focused row. */
export function handleDesktopPendingRowEditEvent(
  event: Event,
  items: readonly QueuedMessage[],
  onEditRow?: (msg: QueuedMessage, entryMode: "insert" | "normal") => void,
): void {
  if (!onEditRow) return;
  const detail = (event as CustomEvent<{ entryMode?: "insert" | "normal" }>).detail;
  const item = event.target instanceof Element
    ? event.target.closest<HTMLElement>("[data-desktop-item]")
    : null;
  const id = item?.dataset.desktopItem;
  if (!id) return;
  const found = items.find((entry) => entry.id === id);
  if (!found || found.id.startsWith("opt-") || found.status !== undefined) return;
  event.preventDefault();
  onEditRow(found, detail?.entryMode ?? "normal");
}
