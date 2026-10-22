import { useEffect, useRef, useState } from "react";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  FormControlLabel,
  InputAdornment,
  ListItemText,
  MenuItem,
  MenuList,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { Search } from "@mui/icons-material";
import { InAppRoot } from "./SessionFolderUi";
import { ConfirmSheet } from "./Sheet";
import { DirectoryWorkbench } from "./DirectoryWorkbench";
import { notify, trashItems, useStoreSelector } from "./store";
import {
  effectiveSessionFolder,
  sessionFolderById,
  sessionFolderLocation,
  type SessionFoldersValue,
} from "./sessionFolders";
import { deleteSelectedSessions, directorySessions } from "./directorySessions";
import { displayedSessionOrder } from "./sessionTree";
import type { SessionMeta } from "./protocol";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { useDialogFocus } from "./useDialogInputFocus";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";

export function DirectorySessionsSheet(
  { id, onClose }: { id: string; onClose: () => void },
): React.JSX.Element {
  const tree = useStoreSelector((state) => state.sessionFolders);
  const sessions = useStoreSelector((state) => state.sessions);
  const deleting = useStoreSelector((state) => state.deletingSessionIds);
  return (
    <DirectorySessionsView
      id={id}
      onClose={onClose}
      tree={tree}
      sessions={sessions}
      deleting={deleting}
      removeSession={(sessionId) => trashItems({ sessions: [sessionId] })}
      onBackgroundFailure={(count) =>
        notify(
          `${count} sessions could not move to Trash. Check Sessions and retry.`,
        )}
    />
  );
}

export function DirectorySessionsView(
  { id, onClose, tree, sessions, deleting, removeSession, onBackgroundFailure }:
    {
      id: string;
      onClose: () => void;
      tree: SessionFoldersValue;
      sessions: readonly SessionMeta[];
      deleting: ReadonlySet<string>;
      removeSession: (id: string) => Promise<void>;
      onBackgroundFailure: (count: number) => void;
    },
): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  const folder = sessionFolderById(tree, id);
  const [descendants, setDescendants] = useState(false);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirm, setConfirm] = useState<SessionMeta[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [failures, setFailures] = useState<Record<string, string>>({});
  const [submitted, setSubmitted] = useState<Set<string>>(new Set());
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const anchor = useRef<string | null>(null);
  const content = useRef<HTMLDivElement>(null);
  useDialogFocus(
    () => content.current?.querySelector<HTMLElement>("input"),
    desktop,
  );
  const members = directorySessions(
    displayedSessionOrder(sessions),
    tree,
    id,
    descendants,
  );
  const query = search.trim().toLocaleLowerCase();
  const visible = members.filter((session) =>
    `${session.title} ${session.id} ${session.provider}`.toLocaleLowerCase()
      .includes(query)
  );
  const chosen = members.filter((session) => selected.has(session.id));
  const actionable = chosen.filter((session) =>
    !session.closing && !deleting.has(session.id)
  );
  const pending = sessions.filter((session) =>
    submitted.has(session.id) && (session.closing || deleting.has(session.id))
  );
  const retained = sessions.filter((session) =>
    submitted.has(session.id) && !session.closing && !deleting.has(session.id)
  );
  const deleted =
    [...submitted].filter((sessionId) =>
      !sessions.some((session) => session.id === sessionId)
    ).length;
  const toggle = (sessionId: string, range: boolean) => {
    const previousAnchor = anchor.current;
    setSelected((current) => {
      const next = new Set(current);
      const start = visible.findIndex((session) =>
        session.id === previousAnchor
      );
      const end = visible.findIndex((session) => session.id === sessionId);
      if (range && start >= 0 && end >= 0) {
        for (
          const session of visible.slice(
            Math.min(start, end),
            Math.max(start, end) + 1,
          )
        ) next.add(session.id);
      } else if (next.has(sessionId)) next.delete(sessionId);
      else next.add(sessionId);
      return next;
    });
    anchor.current = sessionId;
  };
  const remove = async () => {
    if (!confirm || busy || !folder) return;
    const ids = confirm.map((session) => session.id).filter((sessionId) =>
      sessions.some((session) =>
        session.id === sessionId && !session.closing
      ) && !deleting.has(sessionId)
    );
    setConfirm(null);
    setBusy(true);
    setFailures({});
    const errors = await deleteSelectedSessions(ids, removeSession);
    if (!mounted.current) {
      const count = Object.keys(errors).length;
      if (count > 0) onBackgroundFailure(count);
      return;
    }
    setFailures(errors);
    setSubmitted(new Set(ids));
    setBusy(false);
  };
  useConfirmEnter(confirm !== null, () => void remove());
  return (
    <InAppRoot>
      <DirectoryWorkbench
        onClose={onClose}
        title={`${folder?.name ?? "Directory"} · Select sessions`}
        actions={
          <Stack
            direction="row"
            spacing={1}
            alignItems="center"
            sx={{ width: "100%" }}
          >
            <Typography variant="body2" sx={{ flex: 1 }} aria-live="polite">
              {chosen.length} selected
            </Typography>
            <Button onClick={onClose}>Done</Button>
            <Button
              color="error"
              variant="contained"
              disabled={busy || actionable.length === 0 || !folder}
              onClick={() => setConfirm([...actionable])}
            >
              Move to Trash ({actionable.length})
            </Button>
          </Stack>
        }
      >
        <Stack
          ref={content}
          spacing={1.25}
          sx={{
            height: desktop
              ? `min(65dvh, ${Math.min(580, 250 + visible.length * 64)}px)`
              : "100%",
            flex: 1,
            minHeight: 0,
          }}
        >
          {!folder && (
            <Alert severity="warning">
              This directory was removed. No sessions will be deleted here.
            </Alert>
          )}
          <Typography variant="body2" color="text.secondary">
            Select sessions to move to Trash. Directories and Drafts are kept.
          </Typography>
          <FormControlLabel
            control={
              <Checkbox
                checked={descendants}
                disabled={busy}
                onChange={(_, checked) => {
                  setDescendants(checked);
                  setSelected(new Set());
                  anchor.current = null;
                }}
              />
            }
            label="Include subdirectories"
          />
          <TextField
            size="small"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search sessions"
            slotProps={{
              htmlInput: { "aria-label": "Search sessions" },
              input: {
                startAdornment: (
                  <InputAdornment position="start">
                    <Search fontSize="small" />
                  </InputAdornment>
                ),
              },
            }}
          />
          <Stack direction="row" spacing={1} alignItems="center">
            <Button
              disabled={busy || !folder || visible.length === 0}
              onClick={() =>
                setSelected((current) =>
                  new Set([
                    ...current,
                    ...visible.filter((session) =>
                      !session.closing && !deleting.has(session.id)
                    ).map((session) => session.id),
                  ])
                )}
            >
              Select {query
                ? "results"
                : descendants
                ? "all in subtree"
                : "all in directory"} ({visible.length})
            </Button>
            <Button
              disabled={busy || chosen.length === 0}
              onClick={() => setSelected(new Set())}
            >
              Clear
            </Button>
          </Stack>
          {chosen.some((session) => !visible.includes(session)) && (
            <Typography variant="caption">
              {chosen.filter((session) => !visible.includes(session)).length}
              {" "}
              selected outside this search
            </Typography>
          )}
          {(busy || submitted.size > 0) && (
            <Typography role="status" variant="body2">
              {busy
                ? "Moving to Trash…"
                : `${deleted} moved to Trash${
                  pending.length
                    ? ` · ${pending.length} waiting for the environment to stop`
                    : ""
                }${
                  retained.length
                    ? ` · ${retained.length} kept — review and retry`
                    : ""
                }`}
            </Typography>
          )}
          {Object.keys(failures).length > 0 && (
            <Alert severity="error">
              {Object.keys(failures).length}{" "}
              deletion requests failed. Those sessions remain selected; retry
              after checking their errors.
            </Alert>
          )}
          <Box
            sx={{
              overflowY: "auto",
              flex: 1,
              minHeight: 0,
              overscrollBehavior: "contain",
            }}
          >
            <MenuList aria-label="Directory sessions" disablePadding>
              {visible.map((session) => (
                <MenuItem
                  key={session.id}
                  role="menuitemcheckbox"
                  aria-checked={selected.has(session.id)}
                  disabled={busy || Boolean(session.closing) ||
                    deleting.has(session.id) || !folder}
                  onClick={(event) => toggle(session.id, event.shiftKey)}
                  sx={{ px: 0.5, py: 1, alignItems: "flex-start" }}
                >
                  <Checkbox
                    checked={selected.has(session.id)}
                    tabIndex={-1}
                    disableRipple
                    sx={{ pointerEvents: "none" }}
                    slotProps={{ input: { "aria-hidden": true } }}
                  />
                  <ListItemText
                    primary={session.title || session.id}
                    secondary={failures[session.id] ??
                      `${
                        descendants
                          ? `${
                            sessionFolderLocation(
                              tree,
                              effectiveSessionFolder(session, tree),
                            )
                          } · `
                          : ""
                      }${
                        session.machine_id ? `${session.machine_id} · ` : ""
                      }${session.provider} · ${
                        session.closing || deleting.has(session.id)
                          ? "Stopping…"
                          : session.status
                      }`}
                    slotProps={{
                      primary: {
                        sx: { whiteSpace: "normal", overflowWrap: "anywhere" },
                      },
                      secondary: {
                        sx: {
                          whiteSpace: "normal",
                          color: failures[session.id]
                            ? "error.main"
                            : undefined,
                        },
                      },
                    }}
                  />
                </MenuItem>
              ))}
            </MenuList>
            {visible.length === 0 && (
              <Typography color="text.secondary" sx={{ p: 2 }}>
                {query ? "No matching sessions." : "No sessions in this scope."}
              </Typography>
            )}
          </Box>
        </Stack>
      </DirectoryWorkbench>
      <ConfirmSheet
        portal={false}
        open={confirm !== null}
        onClose={() => setConfirm(null)}
        title={`Move ${confirm?.length ?? 0} sessions to Trash?`}
        actions={
          <>
            <Button onClick={() => setConfirm(null)}>
              Cancel<Kbd keys="Esc" />
            </Button>
            <Button
              color="error"
              variant="contained"
              disabled={busy || !folder}
              onClick={() => void remove()}
            >
              Move {confirm?.length ?? 0} to Trash<Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
            </Button>
          </>
        }
      >
        <Stack spacing={1}>
          <Typography>
            They can be restored from Trash with their transcripts and
            worktrees. A working session is not moved; stop it first.
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {folder?.name}
            {descendants
              ? " including subdirectories"
              : " · direct sessions only"}. Directories and Drafts stay in
            place.
          </Typography>
          <Box
            component="ul"
            sx={{ m: 0, pl: 3, maxHeight: "24dvh", overflow: "auto" }}
          >
            {confirm?.map((session) => (
              <li key={session.id}>
                {session.title || session.id}
                <Typography
                  variant="caption"
                  display="block"
                  color="text.secondary"
                >
                  {session.id}
                </Typography>
              </li>
            ))}
          </Box>
        </Stack>
      </ConfirmSheet>
    </InAppRoot>
  );
}
