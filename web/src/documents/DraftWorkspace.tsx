import { defaultDraftTitle } from "./defaultDraftTitle";
import {
  Alert,
  AppBar,
  Box,
  Button,
  Divider,
  Drawer,
  IconButton,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
  Stack,
  Tab,
  Tabs,
  TextField,
  Toolbar,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  Add,
  ArrowBack,
  ChevronRight,
  Close,
  CreateNewFolderOutlined,
  DescriptionOutlined,
  ExpandMore,
  FolderOutlined,
  Menu as MenuIcon,
  MoreHoriz,
  RestoreFromTrashOutlined,
} from "@mui/icons-material";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import {
  DesktopCommandProvider,
  useDesktopCommand,
} from "../desktop/commands/DesktopCommandProvider";
import { DesktopWorkspaceProvider } from "../desktop/DesktopWorkspaceController";
import { EditorExtensionsCommand } from "../editorExtensions/EditorExtensionsDialog";
import {
  FolderNameShell,
  FolderPickerShell,
  SESSION_FOLDER_SHEET_HOST,
} from "../SessionFolderUi";
import { ConfirmSheet } from "../Sheet";
import { DesktopDraftDestinationPicker } from "../desktop/DesktopDraftDestinationPicker";
import { buildSessionTree, displayedSessionOrder } from "../sessionTree";
import { setActiveSessionId } from "../controlPlane";
import { useStoreSelector } from "../store";
import { useBootReady } from "../useBootReady";
import { isImeKeyEvent } from "../imeKey";
import { DraftEditor, type DraftFlush } from "./DraftEditor";
import {
  DRAFT_DRAG_TYPE,
  type DraftChange,
  draftFolderTree,
  draftLocation,
  type DraftMetadata,
} from "./model";
import { draftRepository, useDraftLibrary } from "./store";
import { leaveDrafts, openDrafts } from "./navigation";
import { documentNotice } from "./DocumentNotifications";
import { copyDraftToSession } from "./transfer";

export function DraftWorkspace(
  { id }: { id: string | null },
): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  return desktop
    ? (
      <DesktopWorkspaceProvider>
        <DesktopCommandProvider>
          <EditorExtensionsCommand />
          <DraftWorkspaceBody id={id} desktop />
        </DesktopCommandProvider>
      </DesktopWorkspaceProvider>
    )
    : <DraftWorkspaceBody id={id} desktop={false} />;
}

function DraftCommands(
  { create, close }: { create: () => void; close: () => void },
): null {
  useDesktopCommand(
    useMemo(
      () => ({
        id: "draft.new",
        title: "New draft document",
        group: "Drafts",
        run: create,
      }),
      [create],
    ),
  );
  useDesktopCommand(
    useMemo(
      () => ({
        id: "draft.sessions",
        title: "Open Sessions",
        group: "Drafts",
        run: close,
      }),
      [close],
    ),
  );
  return null;
}

function DraftWorkspaceBody(
  { id, desktop }: { id: string | null; desktop: boolean },
): React.JSX.Element {
  const library = useDraftLibrary();
  const canvas = useRef<HTMLDivElement>(null);
  const [availableWidth, setAvailableWidth] = useState(globalThis.innerWidth);
  useLayoutEffect(() => {
    const element = canvas.current;
    if (!element) return undefined;
    const measure = (): void => setAvailableWidth(element.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const inlineSidebar = desktop && availableWidth >= 820;
  const [drawer, setDrawer] = useState(false);
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [folder, setFolder] = useState<string | null>(null);
  const [trash, setTrash] = useState(false);
  const [tabs, setTabs] = useState<readonly string[]>(id ? [id] : []);
  const [menu, setMenu] = useState<
    { entry: DraftMetadata; anchor: HTMLElement } | null
  >(null);
  const [naming, setNaming] = useState<
    { kind: "folder" | "rename"; entry?: DraftMetadata } | null
  >(null);
  const [moving, setMoving] = useState<DraftMetadata | null>(null);
  const [deleting, setDeleting] = useState<DraftMetadata | null>(null);
  const [destination, setDestination] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [drop, setDrop] = useState<string | null>(null);
  const [navWidth, setNavWidth] = useState(280);
  const [error, setError] = useState<string | null>(null);
  const beforeLeave = useRef<DraftFlush>(() => Promise.resolve());
  const autoExpand = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const active = library.entries.find((entry) => entry.id === id);
  const tree = useMemo(() => draftFolderTree(library.entries), [
    library.entries,
  ]);
  useBootReady(library.loaded && id === null);
  const ancestorIds = (() => {
    const ids: string[] = [];
    let parent = active?.parent_id;
    while (parent && !ids.includes(parent)) {
      ids.push(parent);
      parent = library.entries.find((entry) => entry.id === parent)?.parent_id;
    }
    return ids.join("/");
  })();
  useEffect(() => {
    if (!id) return;
    setTabs((previous) => previous.includes(id) ? previous : [...previous, id]);
    setCollapsed((previous) => {
      const next = new Set(previous);
      for (const parent of ancestorIds.split("/")) next.delete(parent);
      return next.size === previous.size ? previous : next;
    });
  }, [id, ancestorIds]);
  useEffect(() => () => clearTimeout(autoExpand.current), []);
  const attempt = (action: () => Promise<void>): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    void action().catch((cause: unknown) =>
      setError(
        cause instanceof Error ? cause.message : "Draft operation failed",
      )
    ).finally(() => setBusy(false));
  };
  const navigate = async (next: string | null): Promise<void> => {
    await beforeLeave.current();
    setDrawer(false);
    setTrash(false);
    openDrafts(next ?? undefined);
  };
  const create = (): void =>
    attempt(async () => {
      await beforeLeave.current();
      const newId = await draftRepository().create(defaultDraftTitle(), folder);
      setDrawer(false);
      setTrash(false);
      openDrafts(newId);
    });
  const leave = (): void =>
    attempt(async () => {
      await beforeLeave.current();
      leaveDrafts();
    });
  const change = async (
    entry: DraftMetadata,
    action: DraftChange,
  ): Promise<void> => {
    if (entry.id === id) await beforeLeave.current();
    const owner = draftRepository().document(entry.id);
    await owner.refresh();
    await owner.change(action);
    await owner.whenSynced();
  };
  const move = (source: string, parent_id: string | null): void =>
    attempt(async () => {
      const entry = library.entries.find((item) => item.id === source);
      if (!entry || entry.parent_id === parent_id) return;
      await change(entry, { type: "move", parent_id });
      setMoving(null);
    });
  const dropProps = (target: string | null) => ({
    onDragOver: (event: React.DragEvent) => {
      if (!event.dataTransfer.types.includes(DRAFT_DRAG_TYPE)) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      if (drop !== (target ?? "root")) {
        setDrop(target ?? "root");
        clearTimeout(autoExpand.current);
        if (target) {
          autoExpand.current = setTimeout(() =>
            setCollapsed((old) => {
              const next = new Set(old);
              next.delete(target);
              return next;
            }), 500);
        }
      }
    },
    onDragLeave: (event: React.DragEvent) => {
      if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
        setDrop(null);
        clearTimeout(autoExpand.current);
      }
    },
    onDrop: (event: React.DragEvent) => {
      const source = event.dataTransfer.getData(DRAFT_DRAG_TYPE);
      if (!source) return;
      event.preventDefault();
      event.stopPropagation();
      setDrop(null);
      clearTimeout(autoExpand.current);
      move(source, target);
    },
  });
  const needle = query.trim().toLocaleLowerCase();
  const rows: { entry: DraftMetadata; depth: number }[] = [];
  const visible = library.entries.filter((entry) => entry.deleted === trash);
  const sort = (a: DraftMetadata, b: DraftMetadata): number =>
    Number(b.kind === "folder") - Number(a.kind === "folder") ||
    a.title.localeCompare(b.title) || a.id.localeCompare(b.id);
  if (needle || trash) {
    rows.push(
      ...visible.filter((entry) =>
        `${entry.title} ${draftLocation(library.entries, entry.parent_id)}`
          .toLocaleLowerCase().includes(needle)
      ).sort(sort).map((entry) => ({ entry, depth: 0 })),
    );
  } else {
    const seen = new Set<string>();
    const visit = (parent: string | null, depth: number): void => {
      for (
        const entry of visible.filter((item) => item.parent_id === parent).sort(
          sort,
        )
      ) {
        if (seen.has(entry.id)) continue;
        seen.add(entry.id);
        rows.push({ entry, depth });
        if (entry.kind === "folder" && !collapsed.has(entry.id)) {
          visit(entry.id, depth + 1);
        }
      }
    };
    visit(null, 0);
  }
  const sidebar = (
    <Stack sx={{ height: "100%", minHeight: 0 }}>
      <Toolbar variant="dense" sx={{ px: 1, gap: 0.5 }} {...dropProps(null)}>
        <Button color="inherit" startIcon={<ArrowBack />} onClick={leave}>
          Sessions
        </Button>
        <Box sx={{ flex: 1 }} />
        <Tooltip title="New draft">
          <IconButton disabled={busy} aria-label="New draft" onClick={create}>
            <Add />
          </IconButton>
        </Tooltip>
        <Tooltip title="New folder">
          <IconButton
            aria-label="New folder"
            onClick={() => flushSync(() => setNaming({ kind: "folder" }))}
          >
            <CreateNewFolderOutlined />
          </IconButton>
        </Tooltip>
      </Toolbar>
      <TextField
        size="small"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Find drafts…"
        inputProps={{ "aria-label": "Find drafts" }}
        sx={{ mx: 1, mb: 1 }}
      />
      <ListItemButton
        selected={!trash && folder === null}
        onClick={() => {
          setFolder(null);
          setTrash(false);
        }}
        {...dropProps(null)}
        sx={{ bgcolor: drop === "root" ? "action.selected" : undefined }}
      >
        <ListItemText
          primary="Drafts"
          secondary={drop === "root" ? "Move to top level" : undefined}
        />
      </ListItemButton>
      <List
        role="tree"
        aria-label={trash ? "Draft trash" : "Draft documents"}
        sx={{ flex: 1, overflowY: "auto", minHeight: 0, py: 0 }}
        onKeyDown={(event) => {
          if (
            isImeKeyEvent(event.nativeEvent) || event.ctrlKey ||
            event.metaKey || event.altKey
          ) return;
          const items = [
            ...event.currentTarget.querySelectorAll<HTMLElement>(
              "[role=treeitem]",
            ),
          ];
          const index = items.indexOf(document.activeElement as HTMLElement);
          const next =
            (event.key === "ArrowDown" || desktop && event.key === "j")
              ? index + 1
              : (event.key === "ArrowUp" || desktop && event.key === "k")
              ? index - 1
              : event.key === "Home"
              ? 0
              : (event.key === "End" || desktop && event.key === "G")
              ? items.length - 1
              : -1;
          if (next >= 0 && next < items.length) {
            event.preventDefault();
            items[next]?.focus();
          }
        }}
      >
        {rows.map(({ entry, depth }) => (
          <Box key={entry.id} sx={{ position: "relative" }}>
            <ListItemButton
              role="treeitem"
              aria-level={depth + 1}
              aria-selected={entry.id === id}
              {...(entry.kind === "folder"
                ? {
                  "aria-expanded": !collapsed.has(entry.id),
                  ...dropProps(entry.id),
                }
                : {})}
              selected={entry.id === id ||
                entry.kind === "folder" && folder === entry.id}
              draggable={desktop && !trash}
              onDragStart={(event) => {
                event.dataTransfer.setData(DRAFT_DRAG_TYPE, entry.id);
                event.dataTransfer.effectAllowed = "copyMove";
              }}
              onClick={() => {
                if (trash) {
                  setMenu({
                    entry,
                    anchor: document.activeElement as HTMLElement,
                  });
                  return;
                }
                if (entry.kind === "folder") {
                  setFolder(entry.id);
                  setCollapsed((old) => {
                    const next = new Set(old);
                    if (next.has(entry.id)) {
                      next.delete(entry.id);
                    } else next.add(entry.id);
                    return next;
                  });
                } else {attempt(() =>
                    navigate(entry.id)
                  );}
              }}
              onKeyDown={(event) => {
                if (
                  entry.kind !== "folder" || isImeKeyEvent(event.nativeEvent) ||
                  event.ctrlKey || event.metaKey || event.altKey
                ) return;
                if (
                  ["ArrowRight", "ArrowLeft", ...(desktop ? ["h", "l"] : [])]
                    .includes(event.key)
                ) {
                  event.preventDefault();
                  setCollapsed((old) => {
                    const next = new Set(old);
                    if (event.key === "ArrowLeft" || event.key === "h") {
                      next.add(entry.id);
                    } else next.delete(entry.id);
                    return next;
                  });
                }
              }}
              sx={{
                pl: 1 + depth * 1.5,
                pr: 5,
                minHeight: desktop ? "2.35rem" : 44,
                bgcolor: drop === entry.id ? "action.selected" : undefined,
              }}
            >
              <ListItemIcon sx={{ minWidth: "1.75rem" }}>
                {entry.kind === "folder"
                  ? collapsed.has(entry.id)
                    ? <ChevronRight fontSize="small" />
                    : <ExpandMore fontSize="small" />
                  : <DescriptionOutlined fontSize="small" />}
              </ListItemIcon>
              <ListItemText
                primary={entry.title}
                secondary={needle || trash
                  ? draftLocation(library.entries, entry.parent_id)
                  : undefined}
                slotProps={{
                  primary: { noWrap: true, fontSize: "0.95rem" },
                  secondary: { noWrap: true },
                }}
              />
            </ListItemButton>
            <IconButton
              size="small"
              aria-label={`Actions for ${entry.title}`}
              onClick={(event) =>
                setMenu({ entry, anchor: event.currentTarget })}
              sx={{
                position: "absolute",
                right: 2,
                top: 0,
                height: "100%",
                width: desktop ? "2.25rem" : 44,
                padding: desktop ? "0.3rem" : undefined,
              }}
            >
              <MoreHoriz fontSize="small" />
            </IconButton>
          </Box>
        ))}
        {rows.length === 0 && (
          <Typography color="text.secondary" variant="body2" sx={{ p: 2 }}>
            {needle
              ? "No matching drafts"
              : trash
              ? "Trash is empty"
              : "Create a draft to start writing."}
          </Typography>
        )}
      </List>
      <Divider />
      <SessionDropTargets
        beforeLeave={beforeLeave}
        disabled={busy}
        onError={setError}
      />
      <ListItemButton
        selected={trash}
        onClick={() => setTrash((value) => !value)}
        sx={{ flex: "0 0 auto" }}
      >
        <ListItemIcon>
          <RestoreFromTrashOutlined />
        </ListItemIcon>
        <ListItemText primary="Trash" />
      </ListItemButton>
    </Stack>
  );
  return (
    <Stack
      ref={canvas}
      sx={{
        height: "100%",
        minHeight: 0,
        bgcolor: "background.default",
        pt: desktop ? 0 : "var(--cowboy-system-top-clearance)",
        pb: desktop ? 0 : "var(--kb-inset, 0px)",
      }}
      data-draft-workspace
    >
      {desktop && <DraftCommands create={create} close={leave} />}
      <Box sx={{ display: "flex", flex: 1, minHeight: 0 }}>
        {inlineSidebar
          ? (
            <Box
              component="nav"
              aria-label="Draft library"
              data-desktop-pane="sessions"
              data-desktop-region="sessions.list"
              tabIndex={-1}
              sx={{
                width: navWidth,
                minWidth: 190,
                maxWidth: "45%",
                borderRight: 1,
                borderColor: "divider",
                flexShrink: 0,
                position: "relative",
              }}
            >
              {sidebar}
              <Box
                role="separator"
                aria-label="Resize draft sidebar"
                aria-orientation="vertical"
                aria-valuenow={navWidth}
                tabIndex={0}
                onKeyDown={(e) => {
                  if (
                    isImeKeyEvent(e.nativeEvent) || e.ctrlKey || e.metaKey ||
                    e.altKey
                  ) return;
                  if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                    e.preventDefault();
                    setNavWidth((n) =>
                      Math.max(
                        190,
                        Math.min(500, n + (e.key === "ArrowLeft" ? -20 : 20)),
                      )
                    );
                  }
                }}
                onPointerDown={(e) => {
                  if (e.button !== 0) return;
                  e.preventDefault();
                  const start = e.clientX;
                  const width = navWidth;
                  e.currentTarget.setPointerCapture(e.pointerId);
                  const el = e.currentTarget;
                  const move = (event: PointerEvent): void =>
                    setNavWidth(
                      Math.max(
                        190,
                        Math.min(500, width + event.clientX - start),
                      ),
                    );
                  const stop = (): void => {
                    el.removeEventListener("pointermove", move);
                    el.removeEventListener("lostpointercapture", stop);
                  };
                  el.addEventListener("pointermove", move);
                  el.addEventListener("lostpointercapture", stop, {
                    once: true,
                  });
                }}
                sx={{
                  position: "absolute",
                  right: -4,
                  top: 0,
                  bottom: 0,
                  width: 8,
                  cursor: "col-resize",
                  zIndex: 2,
                }}
              />
            </Box>
          )
          : (
            <Drawer
              open={drawer}
              onClose={() => setDrawer(false)}
              slotProps={{
                paper: {
                  sx: {
                    width: "min(88vw, 360px)",
                    pt: "var(--cowboy-system-top-clearance)",
                    pb: "env(safe-area-inset-bottom)",
                  },
                },
              }}
            >
              {sidebar}
            </Drawer>
          )}
        <Stack
          sx={{ flex: 1, minWidth: 0, minHeight: 0 }}
          data-desktop-pane={desktop ? "prompt" : undefined}
        >
          <AppBar
            position="static"
            color="transparent"
            elevation={0}
            sx={{ borderBottom: 1, borderColor: "divider" }}
          >
            <Toolbar variant="dense" sx={{ gap: 1, minHeight: 44 }}>
              {!inlineSidebar && (
                <IconButton
                  aria-label="Open draft library"
                  onClick={() => setDrawer(true)}
                >
                  <MenuIcon />
                </IconButton>
              )}
              <Typography noWrap variant="body2" sx={{ flex: 1 }}>
                {active
                  ? draftLocation(library.entries, active.parent_id)
                  : "Drafts"}
              </Typography>
              {id && (
                <Tooltip title="Move draft">
                  <IconButton
                    aria-label="Move draft"
                    onClick={() => {
                      if (active) setMoving(active);
                    }}
                  >
                    <FolderOutlined />
                  </IconButton>
                </Tooltip>
              )}
              <Tooltip title="New draft">
                <IconButton
                  aria-label="New draft"
                  onClick={create}
                  disabled={busy}
                >
                  <Add />
                </IconButton>
              </Tooltip>
            </Toolbar>
          </AppBar>
          {desktop && tabs.length > 0 && (
            <Tabs
              value={id && tabs.includes(id) ? id : false}
              variant="scrollable"
              scrollButtons="auto"
              aria-label="Open drafts"
              onChange={(_, next: string) => attempt(() => navigate(next))}
              sx={{
                minHeight: "2.5rem",
                borderBottom: 1,
                borderColor: "divider",
              }}
            >
              {tabs.filter((tab) =>
                !library.entries.find((entry) => entry.id === tab)?.deleted
              ).map((tab) => (
                <Tab
                  key={tab}
                  value={tab}
                  sx={{ minHeight: "2.5rem", textTransform: "none", py: 0.25 }}
                  label={
                    <Stack direction="row" alignItems="center" spacing={1}>
                      <Typography
                        noWrap
                        variant="body2"
                        sx={{ maxWidth: "18rem" }}
                      >
                        {library.entries.find((entry) => entry.id === tab)
                          ?.title ?? "Draft"}
                      </Typography>
                      <Box
                        component="span"
                        role="button"
                        tabIndex={0}
                        aria-label="Close draft tab"
                        onClick={(event) => {
                          event.stopPropagation();
                          attempt(async () => {
                            await beforeLeave.current();
                            const next = tabs.filter((item) => item !== tab);
                            setTabs(next);
                            if (tab === id) openDrafts(next.at(-1));
                          });
                        }}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            event.stopPropagation();
                            event.currentTarget.click();
                          }
                        }}
                      >
                        <Close
                          sx={{ fontSize: "1rem", verticalAlign: "middle" }}
                        />
                      </Box>
                    </Stack>
                  }
                />
              ))}
            </Tabs>
          )}
          {(error || library.error) && (
            <Alert
              severity="warning"
              onClose={error ? () => setError(null) : undefined}
            >
              {error ?? library.error}
            </Alert>
          )}
          {id
            ? (
              <DraftEditor
                key={id}
                id={id}
                beforeLeave={beforeLeave}
                onCopyToSession={() => setDestination(id)}
              />
            )
            : (
              <Stack
                sx={{
                  flex: 1,
                  p: 3,
                  justifyContent: "center",
                  alignItems: "center",
                  gap: 1.5,
                }}
              >
                <DescriptionOutlined
                  sx={{ fontSize: "3rem", color: "text.secondary" }}
                />
                <Typography variant="h5">Your writing space</Typography>
                <Typography color="text.secondary">
                  Write independently. Bring a draft into a Session when you're
                  ready.
                </Typography>
                <Button variant="contained" onClick={create} disabled={busy}>
                  New draft
                </Button>
                {!desktop && (
                  <Button onClick={() => setDrawer(true)}>Browse drafts</Button>
                )}
              </Stack>
            )}
        </Stack>
      </Box>
      <Box {...{ [SESSION_FOLDER_SHEET_HOST]: "" }} />
      <Menu
        anchorEl={menu?.anchor}
        open={menu !== null}
        onClose={() => setMenu(null)}
      >
        {menu?.entry.deleted
          ? (
            <MenuItem
              onClick={() => {
                const entry = menu.entry;
                setMenu(null);
                attempt(() => change(entry, { type: "restore" }));
              }}
            >
              Restore
            </MenuItem>
          )
          : (
            <>
              <MenuItem
                onClick={() => {
                  const entry = menu!.entry;
                  flushSync(() => {
                    setMenu(null);
                    setNaming({ kind: "rename", entry });
                  });
                }}
              >
                Rename…
              </MenuItem>
              <MenuItem
                onClick={() => {
                  setMoving(menu!.entry);
                  setMenu(null);
                }}
              >
                Move…
              </MenuItem>
              {menu?.entry.kind === "document" && (
                <MenuItem
                  onClick={() => {
                    setDestination(menu.entry.id);
                    setMenu(null);
                  }}
                >
                  Copy to Session…
                </MenuItem>
              )}
              <MenuItem
                onClick={() => {
                  setDeleting(menu!.entry);
                  setMenu(null);
                }}
              >
                Move to Trash
              </MenuItem>
            </>
          )}
      </Menu>
      {naming && (
        <FolderNameShell
          title={naming.kind === "folder" ? "New draft folder" : "Rename draft"}
          initial={naming.entry?.title ?? ""}
          confirmLabel={naming.kind === "folder" ? "Create" : "Rename"}
          onClose={() => setNaming(null)}
          onConfirm={(name) =>
            attempt(async () => {
              if (naming.entry) {
                await change(naming.entry, { type: "rename", title: name });
              } else await draftRepository().create(name, folder, "folder");
              setNaming(null);
            })}
        />
      )}
      {moving && (
        <FolderPickerShell
          rootLabel="Drafts · top level"
          title={`Move ${moving.title}`}
          value={tree}
          current={moving.parent_id}
          exclude={moving.kind === "folder" ? moving.id : null}
          onClose={() => setMoving(null)}
          onPick={(parent) => move(moving.id, parent)}
        />
      )}
      {deleting && (
        <ConfirmSheet
          open
          title={`Move “${deleting.title}” to Trash?`}
          onClose={() => setDeleting(null)}
          actions={
            <>
              <Button
                color="inherit"
                onClick={() => setDeleting(null)}
              >
                Cancel
              </Button>
              <Button
                color="error"
                disabled={busy}
                onClick={() =>
                  attempt(async () => {
                    const entry = deleting;
                    await change(entry, { type: "trash" });
                    setDeleting(null);
                    if (id === entry.id) await navigate(null);
                    documentNotice("Moved to Trash", () =>
                      draftRepository().document(entry.id).change({
                        type: "restore",
                      }));
                  })}
              >
                Move to Trash
              </Button>
            </>
          }
        >
          <Typography>
            {deleting.kind === "folder"
              ? "Move any contents out of this folder first."
              : "You can restore this draft from Trash."}
          </Typography>
        </ConfirmSheet>
      )}
      {destination && (
        <DesktopDraftDestinationPicker
          sourceId=""
          onClose={() => setDestination(null)}
          onPick={(session) =>
            attempt(async () => {
              if (destination === id) await beforeLeave.current();
              await copyDraftToSession(destination, session.id, session.title);
              setDestination(null);
            })}
        />
      )}
    </Stack>
  );
}

function SessionDropTargets(
  { beforeLeave, disabled, onError }: {
    beforeLeave: React.MutableRefObject<DraftFlush>;
    disabled: boolean;
    onError: (message: string) => void;
  },
): React.JSX.Element {
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const folders = useStoreSelector((snapshot) => snapshot.sessionFolders);
  const [expanded, setExpanded] = useState(false);
  const [dragged, setDragged] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const tree = useMemo(
    () => buildSessionTree(displayedSessionOrder(sessions), folders, new Set()),
    [sessions, folders],
  );
  return (
    <Box sx={{ maxHeight: "35%", overflowY: "auto", flexShrink: 0 }}>
      <ListItemButton
        onClick={() => setExpanded((value) => !value)}
        onDragOver={(event) => {
          if (event.dataTransfer.types.includes(DRAFT_DRAG_TYPE)) {
            event.preventDefault();
            setExpanded(true);
          }
        }}
      >
        <ListItemIcon>
          {expanded ? <ExpandMore /> : <ChevronRight />}
        </ListItemIcon>
        <ListItemText
          primary="Sessions"
          secondary="Drop a draft to copy · never sends"
        />
      </ListItemButton>
      {expanded && (
        <List dense sx={{ py: 0 }}>
          {tree.rows.map((row) =>
            row.kind === "session"
              ? (
                <ListItemButton
                  key={row.session.id}
                  disabled={disabled || pending}
                  onClick={() => {
                    void beforeLeave.current().then(() => {
                      setActiveSessionId(row.session.id);
                      leaveDrafts();
                    }).catch((error: Error) => onError(error.message));
                  }}
                  onDragOver={(event) => {
                    if (event.dataTransfer.types.includes(DRAFT_DRAG_TYPE)) {
                      event.preventDefault();
                      event.dataTransfer.dropEffect = "copy";
                      setDragged(row.session.id);
                    }
                  }}
                  onDragLeave={() => setDragged(null)}
                  onDrop={(event) => {
                    event.preventDefault();
                    const source = event.dataTransfer.getData(DRAFT_DRAG_TYPE);
                    setDragged(null);
                    if (!source || pending) return;
                    setPending(true);
                    void beforeLeave.current().then(() =>
                      copyDraftToSession(
                        source,
                        row.session.id,
                        row.session.title,
                      )
                    )
                      .catch((error: Error) => onError(error.message)).finally(
                        () => setPending(false),
                      );
                  }}
                  sx={{
                    pl: 2 + row.depth * 1.5,
                    bgcolor: dragged === row.session.id
                      ? "action.selected"
                      : undefined,
                  }}
                >
                  <ListItemText
                    primary={row.session.title || "Untitled Session"}
                    slotProps={{ primary: { noWrap: true } }}
                  />
                </ListItemButton>
              )
              : row.kind === "folder"
              ? (
                <Typography
                  key={row.folder.id}
                  variant="caption"
                  color="text.secondary"
                  sx={{ display: "block", pl: 2 + row.depth * 1.5 }}
                >
                  {row.folder.name}
                </Typography>
              )
              : null
          )}
          {sessions.length === 0 && (
            <Typography variant="body2" color="text.secondary" sx={{ p: 2 }}>
              Create a Session when you're ready to use a draft.
            </Typography>
          )}
        </List>
      )}
    </Box>
  );
}
