import { useEffect, useState } from "react";
import {
    Alert,
    AppBar,
    Box,
    Button,
    Chip,
    Dialog,
    DialogContent,
    DialogTitle,
    Divider,
    Drawer,
    IconButton,
    List,
    ListItemButton,
    ListItemIcon,
    ListItemText,
    Menu,
    MenuItem,
    Snackbar,
    Stack,
    Switch,
    TextField,
    Toolbar,
    Tooltip,
    Typography,
    useMediaQuery,
    useTheme,
} from "@mui/material";
import type { SxProps, Theme } from "@mui/material";
import {
    Add,
    Circle,
    DeleteOutline,
    DriveFileRenameOutline,
    Menu as MenuIcon,
    MoreVert,
    Settings as SettingsIcon,
} from "@mui/icons-material";
import { Composer } from "./Composer";
import { Transcript } from "./Transcript";
import {
    originLabel,
    PROVIDERS,
    type SessionMeta,
    type SessionOrigin,
    type Status,
} from "./protocol";
import { send, useStore } from "./store";
import { setVimSetting, useVimSetting } from "./vimSetting";
import { ProviderIcon } from "./ProviderIcon";
import { BottomSheet, PortalLauncherButton, ThemeModeControl } from "./_shell";
import type { Mode as ThemeMode } from "./theme";

// Sidebar width: fluid clamp instead of a fixed pixel value so the panel
// grows on wide displays and shrinks on narrow ones without media-query
// staircase steps. 240px is the floor (a list row stays readable at that
// width), 22vw is the natural scale, 360px is the ceiling (any wider and
// the list looks empty next to the transcript). On mobile the sidebar
// becomes a top-anchored Drawer (full width), so there's no separate
// mobile-width to declare.
const SIDEBAR_WIDTH = "clamp(240px, 22vw, 360px)";

// Status is shown as a single color-coded dot (no text label), so the hue has
// to carry the whole meaning. The palette tokens are chosen so the colors read
// the same here and in any future status surface:
//   green  (success) — running: connected, idle, ready for your next prompt
//   amber  (warning) — busy:    a turn is in flight, the agent is working
//   blue   (info)    — starting: process spinning up, not ready yet
//   red    (error)   — exited / crashed: the session is gone
function statusColor(s: Status): string {
    switch (s) {
        case "running":
            return "success.main";
        case "busy":
            return "warning.main";
        case "starting":
            return "info.main";
        default:
            return "error.main";
    }
}

// Human-readable meaning for the dot — surfaced in its tooltip / aria-label,
// since the dot itself is just color. Mirrors the statusColor mapping above.
function statusLabel(s: Status): string {
    switch (s) {
        case "running":
            return "Ready";
        case "busy":
            return "Working…";
        case "starting":
            return "Starting…";
        case "exited":
            return "Exited";
        case "crashed":
            return "Crashed";
    }
}

// One status indicator, shared by the header and the sidebar list so the
// "green = ready, amber = working" code reads identically everywhere. The dot
// encodes state by color; the tooltip + aria-label spell it out for hover and
// assistive tech (touch has no hover, but the wording is also redundant with
// the surrounding chrome).
function StatusDot({
    status,
    sx,
}: {
    status: Status;
    sx?: SxProps<Theme>;
}): React.JSX.Element {
    return (
        <Tooltip title={statusLabel(status)} enterDelay={300}>
            <Circle
                aria-label={statusLabel(status)}
                sx={[
                    { fontSize: 10, flexShrink: 0, color: statusColor(status) },
                    ...(Array.isArray(sx) ? sx : sx ? [sx] : []),
                ]}
            />
        </Tooltip>
    );
}

function originColor(
    o: SessionOrigin | undefined,
): "primary" | "secondary" | "default" {
    switch (o ?? "api") {
        case "zed":
            return "primary";
        case "web":
            return "secondary";
        default:
            return "default";
    }
}

function SessionList({
    sessions,
    activeId,
    onPick,
    onNew,
    onRequestDelete,
    onRequestRename,
}: {
    sessions: SessionMeta[];
    activeId: string | null;
    onPick: (id: string) => void;
    onNew: () => void;
    onRequestDelete: (s: SessionMeta) => void;
    onRequestRename: (s: SessionMeta) => void;
}): React.JSX.Element {
    // Per-row kebab Menu anchor + target. Standard Material list-row
    // pattern: trailing IconButton with MoreVert opens a Menu containing
    // Rename + Delete — two-step gesture (open menu, pick item, confirm
    // dialog) replaces the previous swipe-to-delete, which the user
    // (correctly) flagged as mis-tap-prone.
    const [menuAnchor, setMenuAnchor] = useState<{
        row: SessionMeta;
        el: HTMLElement;
    } | null>(null);
    return (
        <Stack sx={{ height: "100%" }}>
            <Box sx={{ p: 1 }}>
                <Button
                    fullWidth
                    variant="outlined"
                    startIcon={<Add />}
                    onClick={onNew}
                >
                    New session
                </Button>
            </Box>
            <List dense sx={{ flex: 1, overflowY: "auto" }}>
                {sessions.map((s) => (
                    <ListItemButton
                        key={s.id}
                        selected={s.id === activeId}
                        onClick={(): void => onPick(s.id)}
                        sx={{ pr: 0.5 }}
                    >
                        <StatusDot status={s.status} sx={{ mr: 1 }} />
                        <ListItemText
                            primary={
                                <Stack
                                    direction="row"
                                    spacing={0.75}
                                    alignItems="center"
                                    sx={{ minWidth: 0 }}
                                >
                                    <ProviderIcon
                                        provider={s.provider}
                                        sx={{ fontSize: 16, flexShrink: 0 }}
                                    />
                                    <Typography
                                        variant="body2"
                                        noWrap
                                        sx={{ minWidth: 0 }}
                                    >
                                        {s.title}
                                    </Typography>
                                    <Chip
                                        size="small"
                                        label={originLabel(s.origin)}
                                        color={originColor(s.origin)}
                                        sx={{
                                            height: 16,
                                            fontSize: 10,
                                            "& .MuiChip-label": { px: 0.75 },
                                        }}
                                    />
                                </Stack>
                            }
                            secondary={s.cwd}
                            slotProps={{
                                primary: { component: "div" },
                                secondary: {
                                    noWrap: true,
                                    variant: "caption",
                                },
                            }}
                        />
                        <IconButton
                            size="small"
                            edge="end"
                            aria-label={`row actions ${s.id}`}
                            onClick={(e): void => {
                                e.stopPropagation();
                                setMenuAnchor({ row: s, el: e.currentTarget });
                            }}
                            sx={{ ml: 0.5 }}
                        >
                            <MoreVert fontSize="inherit" />
                        </IconButton>
                    </ListItemButton>
                ))}
                {sessions.length === 0 && (
                    <Typography
                        variant="body2"
                        color="text.secondary"
                        sx={{ p: 2, textAlign: "center" }}
                    >
                        No sessions yet.
                    </Typography>
                )}
            </List>
            <Menu
                anchorEl={menuAnchor?.el ?? null}
                open={!!menuAnchor}
                onClose={(): void => setMenuAnchor(null)}
                slotProps={{ paper: { sx: { minWidth: 180 } } }}
            >
                <MenuItem
                    onClick={(): void => {
                        if (menuAnchor) onRequestRename(menuAnchor.row);
                        setMenuAnchor(null);
                    }}
                >
                    <ListItemIcon>
                        <DriveFileRenameOutline fontSize="small" />
                    </ListItemIcon>
                    <ListItemText primary="Rename" />
                </MenuItem>
                <MenuItem
                    onClick={(): void => {
                        if (menuAnchor) onRequestDelete(menuAnchor.row);
                        setMenuAnchor(null);
                    }}
                    sx={{ color: "error.main" }}
                >
                    <ListItemIcon>
                        <DeleteOutline fontSize="small" color="error" />
                    </ListItemIcon>
                    <ListItemText primary="Delete" />
                </MenuItem>
            </Menu>
        </Stack>
    );
}

// Hard-coded workspace choices for v0. Each entry's `value` is what the
// daemon receives as `cwd`; the resolver in supervisor.rs honours absolute
// paths as-is and joins relative ones to `--workspace-root` (defaults to
// `/home/draven`). To expose more roots later, either bump this list or
// fetch a list from the daemon.
const WORKING_DIRS = [
    { value: "columbus", label: "columbus", help: "/home/draven/columbus" },
    { value: "/etc/nixos", label: "/etc/nixos", help: "NixOS host config" },
] as const;

function NewSessionDialog({
    open,
    onClose,
    onCreated,
}: {
    open: boolean;
    onClose: () => void;
    /** Called with the new session's id so the UI can focus it immediately. */
    onCreated: (sessionId: string) => void;
}): React.JSX.Element {
    const [provider, setProvider] = useState<string>(PROVIDERS[0]);
    const [cwd, setCwd] = useState<string>(WORKING_DIRS[0].value);
    return (
        <Dialog open={open} onClose={onClose} fullWidth maxWidth="xs">
            <DialogTitle>New session</DialogTitle>
            <DialogContent>
                <Stack spacing={2} sx={{ mt: 1 }}>
                    <TextField
                        select
                        label="Provider"
                        value={provider}
                        onChange={(e): void => setProvider(e.target.value)}
                    >
                        {PROVIDERS.map((p) => (
                            <MenuItem key={p} value={p}>
                                {p}
                            </MenuItem>
                        ))}
                    </TextField>
                    <TextField
                        select
                        label="Working directory"
                        value={cwd}
                        onChange={(e): void => setCwd(e.target.value)}
                        helperText={
                            WORKING_DIRS.find((w) => w.value === cwd)?.help ??
                            ""
                        }
                    >
                        {WORKING_DIRS.map((w) => (
                            <MenuItem key={w.value} value={w.value}>
                                {w.label}
                            </MenuItem>
                        ))}
                    </TextField>
                    <Button
                        variant="contained"
                        onClick={(): void => {
                            // POST (not the fire-and-forget WS `new_session`) so
                            // we get the assigned id back synchronously and can
                            // focus the new session the moment it's created.
                            void fetch("/api/sessions", {
                                method: "POST",
                                headers: { "content-type": "application/json" },
                                body: JSON.stringify({
                                    provider,
                                    cwd,
                                    origin: "web",
                                }),
                            })
                                .then((r) => (r.ok ? r.json() : null))
                                .then((data: { session_id?: string } | null) => {
                                    if (data?.session_id)
                                        onCreated(data.session_id);
                                })
                                .catch(() => {
                                    // Network/daemon error surfaces via the WS
                                    // error channel; nothing to do here.
                                });
                            onClose();
                        }}
                    >
                        Create
                    </Button>
                </Stack>
            </DialogContent>
        </Dialog>
    );
}

export function App({
    themeMode,
    onSetThemeMode,
}: {
    themeMode: ThemeMode;
    onSetThemeMode: (m: ThemeMode) => void;
}): React.JSX.Element {
    const { connected, sessions, timelines, lastError } = useStore();
    // The error notice is monotonically `seq`-stamped so the same message
    // text triggers the snackbar twice if it happens again. Tracking the
    // `seq` we've shown means we don't re-open after the user dismisses.
    const [shownErrorSeq, setShownErrorSeq] = useState(0);
    const errorOpen = !!lastError && lastError.seq > shownErrorSeq;
    useEffect(() => {
        // No-op on mount; effect exists so future enhancements (e.g. coalescing
        // duplicate messages) have a hook. Keeps the reactive trace explicit.
    }, [lastError?.seq]);
    const theme = useTheme();
    // Sidebar collapse aka "drawer mode". Anything below the `lg` breakpoint
    // (1200px) hides the persistent sidebar and shows a hamburger that opens
    // the Drawer — this catches both iPad orientations (portrait ~820,
    // landscape ~1180), not just phones. The session-list-as-persistent-
    // rail layout reads as cramped on iPad: the transcript pane loses too
    // many columns, and the chip row gets squeezed. Matches Mail.app /
    // Messages on iPad, which also collapse their sidebars in both
    // orientations until the device is wider than ~1200pt.
    const mobile = useMediaQuery(theme.breakpoints.down("lg"));
    const [activeId, setActiveId] = useState<string | null>(null);
    const [drawerOpen, setDrawerOpen] = useState(false);
    const [dialogOpen, setDialogOpen] = useState(false);
    const [settingsOpen, setSettingsOpen] = useState(false);
    // Session targeted for deletion; non-null = the dialog is open. Held in
    // App so the dialog is a single instance instead of one per row, and so
    // its mobile vs desktop shell can react to `bottomSheet` from useMediaQuery.
    const [pendingDelete, setPendingDelete] = useState<SessionMeta | null>(
        null,
    );
    // Same pattern for rename — single dialog instance prefilled with the
    // session's current title; Mobile/desktop shell split mirrors the rest.
    const [pendingRename, setPendingRename] = useState<SessionMeta | null>(
        null,
    );
    // Default to the first session once one exists.
    const active =
        sessions.find((s) => s.id === activeId) ?? sessions[0] ?? null;

    function pick(id: string): void {
        setActiveId(id);
        setDrawerOpen(false);
    }

    const list = (
        <SessionList
            sessions={sessions}
            activeId={active?.id ?? null}
            onPick={pick}
            onNew={(): void => setDialogOpen(true)}
            onRequestDelete={(s): void => setPendingDelete(s)}
            onRequestRename={(s): void => setPendingRename(s)}
        />
    );

    // Brand toolbar for the desktop sidebar. Uses MUI's own `<Toolbar>` so
    // its height comes from `theme.mixins.toolbar` (responsive across
    // breakpoints) and stays in lockstep with the AppBar's Toolbar on the
    // right pane — no hardcoded pixel value. On mobile the sidebar is a
    // Drawer and the brand goes in the AppBar instead.
    const sidebarHeader = (
        <Toolbar
            variant="dense"
            sx={{
                borderBottom: 1,
                borderColor: "divider",
                flexShrink: 0,
                gap: 0.5,
            }}
        >
            {/* Portal launcher — the app's absolute top-left; self-hides when not hosted. */}
            <PortalLauncherButton edge="start" size="small" />
            <Typography variant="subtitle1" noWrap sx={{ fontWeight: 500 }}>
                cowboy
            </Typography>
        </Toolbar>
    );

    return (
        <Box sx={{ display: "flex", height: "100%", width: "100%" }}>
            {mobile ? (
                <Drawer
                    anchor="top"
                    open={drawerOpen}
                    onClose={(): void => setDrawerOpen(false)}
                    slotProps={{
                        paper: {
                            sx: {
                                maxHeight: "80vh",
                                borderBottomLeftRadius: 16,
                                borderBottomRightRadius: 16,
                                // Slide DOWN from the top rather than in from the left: a
                                // left-anchored drawer fought the iOS back / app-switch
                                // edge-swipe (a swipe to scroll the list kept switching apps).
                                // Opening is a hamburger tap, so there's no edge gesture at all.
                                // pt clears the notch / status bar; side insets clear the
                                // rounded corners / notch in landscape (0 off-device).
                                pt: "max(env(safe-area-inset-top), 8px)",
                                pl: "env(safe-area-inset-left)",
                                pr: "env(safe-area-inset-right)",
                            },
                        },
                    }}
                >
                    {list}
                </Drawer>
            ) : (
                <Stack
                    sx={{
                        width: SIDEBAR_WIDTH,
                        flexShrink: 0,
                        borderRight: 1,
                        borderColor: "divider",
                        height: "100%",
                    }}
                >
                    {sidebarHeader}
                    <Box sx={{ flex: 1, minHeight: 0 }}>{list}</Box>
                </Stack>
            )}

            <Stack sx={{ flex: 1, minWidth: 0 }}>
                <AppBar position="static" color="default" elevation={0}>
                    <Toolbar
                        variant="dense"
                        sx={{ borderBottom: 1, borderColor: "divider" }}
                    >
                        {/* On mobile the sidebar (with its launcher) is hidden, so the
                drawer toggle leads the bar; the launcher moves to the far right,
                after Settings (the app's chosen placement on this bar). */}
                        {mobile && (
                            <IconButton
                                edge="start"
                                onClick={(): void => setDrawerOpen(true)}
                                sx={{ mr: 1 }}
                            >
                                <MenuIcon />
                            </IconButton>
                        )}
                        {active ? (
                            // Status is a leading dot (color = state); the
                            // ProviderIcon already names the agent, so the title
                            // drops the redundant "provider · " prefix the
                            // daemon stores and shows just the cwd (or the
                            // user's custom rename). Full string in the tooltip.
                            <Stack
                                direction="row"
                                alignItems="center"
                                spacing={0.75}
                                sx={{ flex: 1, minWidth: 0 }}
                            >
                                <StatusDot status={active.status} />
                                <ProviderIcon
                                    provider={active.provider}
                                    sx={{ fontSize: 20, flexShrink: 0 }}
                                />
                                <Tooltip title={active.title} enterDelay={400}>
                                    <Typography
                                        variant="subtitle1"
                                        noWrap
                                        sx={{ minWidth: 0 }}
                                    >
                                        {active.title.startsWith(
                                            `${active.provider} · `,
                                        )
                                            ? active.title.slice(
                                                  active.provider.length + 3,
                                              )
                                            : active.title}
                                    </Typography>
                                </Tooltip>
                            </Stack>
                        ) : (
                            // No session: the content pane already says "No
                            // session selected", so the bar shows nothing — no
                            // redundant brand/emoji.
                            <Box sx={{ flex: 1, minWidth: 0 }} />
                        )}
                        {!connected && (
                            <Chip
                                size="small"
                                color="error"
                                label="offline"
                                sx={{ mr: 1 }}
                            />
                        )}
                        <IconButton
                            onClick={(): void => setSettingsOpen(true)}
                            aria-label="settings"
                            title="Settings"
                        >
                            <SettingsIcon />
                        </IconButton>
                        {/* Launcher last — to the right of Settings (self-hides standalone). */}
                        {mobile && <PortalLauncherButton size="small" />}
                    </Toolbar>
                </AppBar>

                {active ? (
                    <>
                        <Transcript
                            sessionId={active.id}
                            timeline={timelines.get(active.id) ?? []}
                            status={active.status}
                        />
                        <Composer
                            sessionId={active.id}
                            status={active.status}
                        />
                    </>
                ) : (
                    // Empty state: relative parent + absolutely-positioned content
                    // centered to the geometric middle of the *whole* right pane
                    // (including the AppBar area), so it reads as the viewport
                    // center even on tall screens. flex-centering would push it
                    // ~24px down (half the AppBar height).
                    <Box sx={{ flex: 1, position: "relative" }}>
                        <Stack
                            spacing={2}
                            alignItems="center"
                            sx={{
                                position: "absolute",
                                top: "50%",
                                left: "50%",
                                transform: "translate(-50%, -50%)",
                                p: 3,
                                width: "max-content",
                                maxWidth: "calc(100% - 48px)",
                            }}
                        >
                            <Typography color="text.secondary">
                                No session selected.
                            </Typography>
                            <Button
                                variant="contained"
                                startIcon={<Add />}
                                onClick={(): void => setDialogOpen(true)}
                            >
                                New session
                            </Button>
                        </Stack>
                    </Box>
                )}
            </Stack>

            <NewSessionDialog
                open={dialogOpen}
                onClose={(): void => setDialogOpen(false)}
                onCreated={(id): void => {
                    // Focus the freshly-created session as soon as the daemon
                    // returns its id; the `sessions` broadcast that adds it to
                    // the list arrives moments later and `active` resolves it.
                    setActiveId(id);
                    setDrawerOpen(false);
                }}
            />
            <DeleteSessionShell
                session={pendingDelete}
                onClose={(): void => setPendingDelete(null)}
                onConfirm={(): void => {
                    if (pendingDelete) {
                        send({
                            type: "delete_session",
                            session_id: pendingDelete.id,
                        });
                    }
                    setPendingDelete(null);
                }}
            />
            <RenameSessionShell
                session={pendingRename}
                onClose={(): void => setPendingRename(null)}
                onConfirm={(title): void => {
                    if (pendingRename) {
                        send({
                            type: "rename_session",
                            session_id: pendingRename.id,
                            title,
                        });
                    }
                    setPendingRename(null);
                }}
            />
            <SettingsShell
                open={settingsOpen}
                onClose={(): void => setSettingsOpen(false)}
                themeMode={themeMode}
                onSetThemeMode={onSetThemeMode}
            />
            <Snackbar
                open={errorOpen}
                autoHideDuration={5000}
                onClose={(): void => setShownErrorSeq(lastError?.seq ?? 0)}
                anchorOrigin={{
                    vertical: "bottom",
                    horizontal: mobile ? "center" : "right",
                }}
                // Lift above the composer's safe-area inset on mobile so the toast
                // doesn't sit underneath the action row.
                sx={{
                    bottom: {
                        xs: "calc(env(safe-area-inset-bottom) + 96px)",
                        sm: 24,
                    },
                }}
            >
                <Alert
                    severity="error"
                    variant="filled"
                    onClose={(): void => setShownErrorSeq(lastError?.seq ?? 0)}
                    sx={{ maxWidth: 480 }}
                >
                    {lastError?.message ?? ""}
                </Alert>
            </Snackbar>
        </Box>
    );
}

// Settings surface. Uses the shared BottomSheet (DetentSheet momentum sheet on
// mobile, centred Dialog on desktop) so it looks/behaves identically to every
// other app's settings + the portal's launcher. Content is the shared
// ThemeModeControl + a small About blurb.
function SettingsShell({
    open,
    onClose,
    themeMode,
    onSetThemeMode,
}: {
    open: boolean;
    onClose: () => void;
    themeMode: ThemeMode;
    onSetThemeMode: (m: ThemeMode) => void;
}): React.JSX.Element {
    const vim = useVimSetting();
    // Vim is desktop-only (ComposerEditor won't load it on touch), so the
    // toggle only appears where a physical keyboard exists.
    const desktop = useMediaQuery("(pointer: fine) and (hover: hover)");
    return (
        <BottomSheet open={open} onClose={onClose} title="Settings">
            <Stack spacing={3}>
                <ThemeModeControl value={themeMode} onChange={onSetThemeMode} />
                {desktop && (
                    <>
                        <Divider />
                        <Stack
                            direction="row"
                            alignItems="center"
                            justifyContent="space-between"
                            spacing={2}
                        >
                            <Stack>
                                <Typography variant="body2">
                                    Vim keybindings
                                </Typography>
                                <Typography
                                    variant="caption"
                                    color="text.secondary"
                                >
                                    Modal editing in the composer
                                </Typography>
                            </Stack>
                            <Switch
                                checked={vim}
                                onChange={(e): void =>
                                    setVimSetting(e.target.checked)
                                }
                                inputProps={{ "aria-label": "Vim keybindings" }}
                            />
                        </Stack>
                    </>
                )}
                <Divider />
                <Stack spacing={0.5}>
                    <Typography variant="overline" color="text.secondary">
                        About
                    </Typography>
                    <Typography variant="body2" color="text.secondary">
                        cowboy v0.1 — multi-agent panel driving Claude Code / Codex over ACP.
                    </Typography>
                </Stack>
            </Stack>
        </BottomSheet>
    );
}

// Responsive delete-session confirmation. Same split as SettingsShell —
// Drawer anchor=bottom (iOS-style bottom sheet with drag handle) on phones
// and portrait iPad; centred Dialog on desktop and landscape iPad. Replaces
// the browser `window.confirm` which is unstyled, blocks input, and can't
// adapt its layout per viewport.
function DeleteSessionShell({
    session,
    onClose,
    onConfirm,
}: {
    session: SessionMeta | null;
    onClose: () => void;
    onConfirm: () => void;
}): React.JSX.Element | null {
    if (!session) return null;
    const surface = originLabel(session.origin);
    return (
        <BottomSheet
            open
            onClose={onClose}
            title={`Delete this ${surface} session?`}
            actions={
                <>
                    <Button onClick={onClose} color="inherit">
                        Cancel
                    </Button>
                    <Button onClick={onConfirm} color="error" variant="contained" autoFocus>
                        Delete
                    </Button>
                </>
            }
        >
            <Typography variant="body2" color="text.secondary">
                Any in-flight turn is cancelled. The agent transcript on this session will be lost.
            </Typography>
        </BottomSheet>
    );
}

// Prefills the textfield with the current title; Save is disabled while empty
// or unchanged (server-side also rejects empty).
function RenameSessionShell({
    session,
    onClose,
    onConfirm,
}: {
    session: SessionMeta | null;
    onClose: () => void;
    onConfirm: (title: string) => void;
}): React.JSX.Element | null {
    const [value, setValue] = useState("");
    useEffect(() => {
        if (session) setValue(session.title);
    }, [session?.id, session?.title]);
    if (!session) return null;
    const trimmed = value.trim();
    const canSave = trimmed.length > 0 && trimmed !== session.title;
    const submit = (): void => {
        if (canSave) onConfirm(trimmed);
    };
    return (
        <BottomSheet
            open
            onClose={onClose}
            title="Rename session"
            actions={
                <>
                    <Button onClick={onClose} color="inherit">
                        Cancel
                    </Button>
                    <Button onClick={submit} variant="contained" disabled={!canSave}>
                        Save
                    </Button>
                </>
            }
        >
            <TextField
                autoFocus
                fullWidth
                label="Title"
                value={value}
                onChange={(e): void => setValue(e.target.value)}
                onKeyDown={(e): void => {
                    if (e.key === "Enter" && !e.shiftKey) {
                        e.preventDefault();
                        submit();
                    }
                }}
                sx={{ mt: 1 }}
                helperText="Shown in the sidebar and the title bar."
            />
        </BottomSheet>
    );
}

