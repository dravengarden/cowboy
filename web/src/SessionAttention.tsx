import { useMemo, useState } from "react";
import { Box, Button, ListItemText, Menu, MenuItem, Tooltip } from "@mui/material";
import PendingActionsOutlined from "@mui/icons-material/PendingActionsOutlined";
import { useCallApproval, useCallApprovalSessionIds } from "./callApproval";
import type { SessionMeta } from "./protocol";
import { sessionAttentionLabel } from "./sessionAttentionModel";
import { effectiveSessionFolder, folderAncestors, projectFolderIndex, type SessionFoldersValue } from "./sessionFolders";

export function SessionActionBadge({ session }: { session: SessionMeta }): React.JSX.Element | null {
  const call = useCallApproval(session.id);
  const label = sessionAttentionLabel(session, call !== null);
  if (!label) return null;
  return <Tooltip title={`${label} · open this session to respond`}>
    <Box component="span" data-session-action-needed role="status" sx={{
      display: "inline-flex", alignItems: "center", gap: 0.35, flexShrink: 0,
      color: "text.primary", fontSize: "0.6875rem", fontWeight: 600,
    }}><PendingActionsOutlined sx={{ fontSize: "1em", color: "warning.main" }} />{label}</Box>
  </Tooltip>;
}

/** The same persistent inbox is available globally and on a collapsed folder.
 * It navigates to the existing decision dock; approval never happens in a row. */
export function SessionAttentionMenu({ sessions, onPick, folder, folders }: {
  sessions: readonly SessionMeta[];
  onPick: (id: string) => void;
  folder?: string;
  folders?: SessionFoldersValue;
}): React.JSX.Element | null {
  const calls = useCallApprovalSessionIds();
  const pending = useMemo(() => {
    const ids = new Set<string>(JSON.parse(calls));
    const projects = folders ? projectFolderIndex(folders) : undefined;
    return sessions.flatMap((session) => {
      const label = sessionAttentionLabel(session, ids.has(session.id));
      if (!label) return [];
      if (folder && folders && projects) {
        const parent = effectiveSessionFolder(session, folders, projects);
        if (parent !== folder && !folderAncestors(folders, parent).includes(folder)) return [];
      }
      return [{ session, label }];
    });
  }, [sessions, calls, folder, folders]);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  if (pending.length === 0) return null;
  return <Box component="span" onClick={(event) => event.stopPropagation()} sx={{ flexShrink: 0 }}>
    <Button size="small" color="inherit" startIcon={<PendingActionsOutlined sx={{ color: "warning.main" }} />}
      data-session-attention-inbox={folder ?? "all"}
      aria-haspopup="menu" aria-expanded={anchor !== null}
      aria-label={`${pending.length} sessions need action${folder ? " in this folder" : ""}`}
      onClick={(event) => setAnchor(event.currentTarget)}
      sx={{ minWidth: 0, minHeight: 44, fontSize: "0.75rem", whiteSpace: "nowrap", "@media (pointer: fine)": { minHeight: 32 } }}>
      {pending.length} need action
    </Button>
    <Menu anchorEl={anchor} open={anchor !== null} onClose={() => setAnchor(null)}>
      {pending.map(({ session, label }) => <MenuItem key={session.id} onClick={() => {
        setAnchor(null);
        onPick(session.id);
      }}><ListItemText primary={session.title} secondary={label} /></MenuItem>)}
    </Menu>
  </Box>;
}
