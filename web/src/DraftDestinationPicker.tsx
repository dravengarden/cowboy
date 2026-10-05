import { useCallback, useState } from "react";
import { Box } from "@mui/material";
import { DriveFileMoveOutlined } from "@mui/icons-material";
import type { SessionMeta } from "./protocol";
import { useStoreSelector } from "./store";
import type { SessionFoldersValue } from "./sessionFolders";
import {
  SessionDestinationTree,
  type SessionDestinationTreeState,
} from "./SessionDestinationTree";
import { Sheet } from "./Sheet";
import { DesktopModal } from "./desktop/DesktopModal";
import { desktopScrollbarSx } from "./desktop/desktopScrollbar";
import { sequentialShortcutAvailability } from "./desktop/commands/shortcutAvailability";
import { useSurfaceProfile } from "./surface/SurfaceProfile";

interface DraftDestinationProps {
  title: string;
  /** Session the draft lives in today (`""` for an independent Draft). */
  sourceId: string;
  /** Folder whose path starts open when there is no source Session. */
  initialFolder?: string | null;
  /** A pick is in flight: rows and dismissal are held. */
  busy?: boolean;
  onPick: (session: SessionMeta) => void;
  onClose: () => void;
}

/** Mounted only while open: the writing surface that opens it never
 * subscribes to unrelated session status, folder or order changes. */
export function DraftDestinationPicker(
  props: DraftDestinationProps,
): React.JSX.Element {
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const folders = useStoreSelector((snapshot) => snapshot.sessionFolders);
  const order = useStoreSelector((snapshot) => snapshot.workspaceOrder);
  const desktop = useSurfaceProfile().kind === "desktop";
  const data = { ...props, sessions, folders, order };
  return desktop
    ? <DraftDestinationModal {...data} />
    : <DraftDestinationSheet {...data} />;
}

type DraftDestinationData = DraftDestinationProps & {
  sessions: readonly SessionMeta[];
  folders: SessionFoldersValue;
  order: readonly string[];
};

/** Touch: the Sessions tree in the compact inset sheet; it scrolls with the
 * sheet body so iOS never nests two momentum scrollers. */
export function DraftDestinationSheet(
  {
    title,
    sourceId,
    initialFolder,
    busy = false,
    onPick,
    onClose,
    sessions,
    folders,
    order,
  }: DraftDestinationData,
): React.JSX.Element {
  return (
    <Sheet
      open
      onClose={busy ? () => {} : onClose}
      title={title}
      mobileDismiss="footer"
      portal
    >
      <Box sx={{ pt: 0.5, pb: 1 }}>
        <SessionDestinationTree
          sessions={sessions}
          folders={folders}
          order={order}
          currentId={sourceId || null}
          {...(initialFolder !== undefined && { initialFolder })}
          busy={busy}
          onPick={onPick}
        />
      </Box>
    </Sheet>
  );
}

/** Desktop: a keyboard-first modal whose shortcut bar is the live legend for
 * the tree's own motions. */
export function DraftDestinationModal(
  {
    title,
    sourceId,
    initialFolder,
    busy = false,
    onPick,
    onClose,
    sessions,
    folders,
    order,
  }: DraftDestinationData,
): React.JSX.Element {
  const [state, setState] = useState<SessionDestinationTreeState>({
    searchFocused: false,
    prefixArmed: false,
  });
  const onStateChange = useCallback(
    (next: SessionDestinationTreeState) => setState(next),
    [],
  );
  const tree = state.searchFocused || state.prefixArmed
    ? "inactive"
    : "available";
  return (
    <DesktopModal
      open
      onClose={busy ? () => {} : onClose}
      title={title}
      description="The draft stays unsent until you send it there."
      icon={<DriveFileMoveOutlined color="primary" />}
      width={720}
      shortcutGroups={[
        {
          label: "Navigate",
          slots: [
            { shortcut: "J/K", label: "Move", availability: tree },
            { shortcut: "H/L", label: "Fold", availability: tree },
          ],
        },
        {
          label: "Go",
          slots: [
            {
              shortcut: "G",
              label: "Prefix",
              availability: sequentialShortcutAvailability({
                scopeAvailable: !state.searchFocused,
                armed: state.prefixArmed,
                prefix: true,
              }),
            },
            {
              shortcut: "G",
              label: "First",
              availability: sequentialShortcutAvailability({
                scopeAvailable: !state.searchFocused,
                armed: state.prefixArmed,
                prefix: false,
              }),
            },
            { shortcut: "Shift+G", label: "Last", availability: tree },
          ],
        },
        {
          slots: [
            {
              shortcut: "/",
              label: "Search",
              availability: state.searchFocused ? "active" : tree,
            },
            {
              shortcut: "Enter",
              label: state.searchFocused ? "To list" : "Choose",
            },
            {
              shortcut: "Esc",
              label: state.searchFocused ? "Leave search" : "Close",
            },
          ],
        },
      ]}
    >
      <Box
        sx={{
          px: 2.25,
          pt: 1.5,
          display: "flex",
          flexDirection: "column",
          height: "min(520px, calc(100vh - 220px))",
          minHeight: 240,
        }}
      >
        <SessionDestinationTree
          sessions={sessions}
          folders={folders}
          order={order}
          currentId={sourceId || null}
          {...(initialFolder !== undefined && { initialFolder })}
          busy={busy}
          onPick={onPick}
          onStateChange={onStateChange}
          listSx={{ overflowY: "auto", pb: 1, ...desktopScrollbarSx }}
        />
      </Box>
    </DesktopModal>
  );
}
