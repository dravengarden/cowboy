import { useEffect, useRef, useState } from "react";
import {
    Alert,
    AppBar,
    Box,
    Button,
    Chip,
    CircularProgress,
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
import { BottomSheet, ThemeModeControl } from "./_shell";
import type { Mode as ThemeMode } from "./theme";

// Desktop sidebar width: a user-draggable pixel width (VSCode-style divider),
// persisted in localStorage. The bounds keep both panes usable — 240px floor
// (a list row stays readable), 480px ceiling (wider and the list looks empty
// next to the transcript). 300px default sits inside the old fluid
// `clamp(240px, 22vw, 360px)` it replaces. Resize is desktop-only: below the
// `lg` breakpoint the sidebar is a full-width top Drawer (touch layout), which
// has no divider — so neither bound nor handle applies there.
const SIDEBAR_MIN = 240;
const SIDEBAR_MAX = 480;
const SIDEBAR_DEFAULT = 300;
const SIDEBAR_WIDTH_KEY = "cowboy:sidebar-width";

function clampSidebarWidth(px: number): number {
    return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, px));
}

// Seed the width from localStorage, re-clamped in case the bounds changed
// since it was stored. Falls back to the default when unset or unparseable.
function readSidebarWidth(): number {
    const raw = globalThis.localStorage?.getItem(SIDEBAR_WIDTH_KEY);
    const n = raw ? Number.parseInt(raw, 10) : Number.NaN;
    return Number.isFinite(n) ? clampSidebarWidth(n) : SIDEBAR_DEFAULT;
}

// The session the user last had focused, so a page reload (or PWA relaunch)
// reopens it instead of snapping back to the top of the list. Just an id; if it
// names a session that no longer exists (deleted elsewhere) the `active`
// derivation falls back to the first session, so no validation is needed here.
const ACTIVE_SESSION_KEY = "cowboy:active-session";

function readActiveSession(): string | null {
    return globalThis.localStorage?.getItem(ACTIVE_SESSION_KEY) ?? null;
}

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
    const extra = Array.isArray(sx) ? sx : sx ? [sx] : [];
    // "busy" is the one *active* state — a turn is in flight — so a static dot
    // reads as idle/stuck (the reported "yellow dot is weird while running").
    // Swap it for a tiny spinner sized to the dot so the row says "working",
    // keeping the amber (warning) hue for continuity with the dot palette. Every
    // other state is a settled condition, so it stays a color-coded dot.
    const indicator =
        status === "busy" ? (
            <CircularProgress
                size={11}
                thickness={6}
                color="warning"
                aria-label={statusLabel(status)}
                sx={[{ flexShrink: 0 }, ...extra]}
            />
        ) : (
            <Circle
                aria-label={statusLabel(status)}
                sx={[
                    { fontSize: 10, flexShrink: 0, color: statusColor(status) },
                    ...extra,
                ]}
            />
        );
    return (
        <Tooltip title={statusLabel(status)} enterDelay={300}>
            {indicator}
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
                        // Keep the trailing kebab off the screen edge: floor a
                        // right inset so it never hugs the rounded corner / iOS
                        // back-swipe edge (ui.md §7), where it was easy to miss.
                        sx={{ pr: "max(env(safe-area-inset-right), 8px)" }}
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
                                        fontSize="medium"
                                        sx={{ flexShrink: 0 }}
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
                            aria-label={`row actions ${s.id}`}
                            onClick={(e): void => {
                                e.stopPropagation();
                                setMenuAnchor({ row: s, el: e.currentTarget });
                            }}
                            // Full-size (≥40px) tap target with a real 24px
                            // glyph. Was size="small" + fontSize="inherit"
                            // (tiny) and edge="end" (negative margin pulling it
                            // flush to the edge) — mis-tap-prone on touch.
                            sx={{ ml: 0.5, width: 40, height: 40, flexShrink: 0 }}
                        >
                            <MoreVert />
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
                        <DriveFileRenameOutline fontSize="medium" />
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
                        <DeleteOutline fontSize="medium" color="error" />
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
    const [activeId, setActiveId] = useState<string | null>(readActiveSession);
    const [drawerOpen, setDrawerOpen] = useState(false);
    const [dialogOpen, setDialogOpen] = useState(false);
    const [settingsOpen, setSettingsOpen] = useState(false);
    // Desktop sidebar width + live-drag flag. Width is a pixel value (not the
    // old fluid clamp) so the divider can set it directly; `resizing` drives
    // the handle's active highlight and a body-wide drag cursor / no-select.
    const [sidebarWidth, setSidebarWidth] = useState<number>(readSidebarWidth);
    const [resizing, setResizing] = useState(false);
    // Persist after a drag settles, not on every pointermove (localStorage is
    // synchronous — writing per pixel would stutter the drag). The ref carries
    // the latest dragged width into pointerup without re-binding listeners.
    const widthRef = useRef(sidebarWidth);
    widthRef.current = sidebarWidth;
    // While dragging the divider, force the col-resize cursor and kill text
    // selection document-wide — otherwise the pointer flickers to a text caret
    // and a fast drag highlights the transcript. Restored on release/unmount.
    useEffect(() => {
        if (!resizing) return undefined;
        const { body } = document;
        const prevCursor = body.style.cursor;
        const prevSelect = body.style.userSelect;
        body.style.cursor = "col-resize";
        body.style.userSelect = "none";
        return (): void => {
            body.style.cursor = prevCursor;
            body.style.userSelect = prevSelect;
        };
    }, [resizing]);
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

    // Persist the *resolved* focus so a reload reopens it. Keyed on `active.id`
    // (not raw `activeId`) so a stale stored id that fell back to sessions[0]
    // gets corrected to what's actually shown. Skip while `active` is null —
    // during the initial load (sessions not yet broadcast) we must not clobber
    // the stored id with null before it has a chance to resolve.
    useEffect(() => {
        if (active) {
            globalThis.localStorage?.setItem(ACTIVE_SESSION_KEY, active.id);
        }
    }, [active]);

    function pick(id: string): void {
        setActiveId(id);
        setDrawerOpen(false);
    }

    // VSCode-style divider drag. Pointer capture keeps move/up events flowing
    // to the handle even when the pointer outruns it, so a fast drag never
    // "drops". Only the primary button starts a resize. Width is derived from
    // the pointer delta off the drag's start, re-clamped each move; the final
    // value is persisted on release (see widthRef rationale above).
    function startResize(e: React.PointerEvent<HTMLDivElement>): void {
        if (e.button !== 0) return;
        e.preventDefault();
        const startX = e.clientX;
        const startWidth = widthRef.current;
        const el = e.currentTarget;
        el.setPointerCapture(e.pointerId);
        setResizing(true);
        const onMove = (ev: PointerEvent): void => {
            setSidebarWidth(clampSidebarWidth(startWidth + (ev.clientX - startX)));
        };
        const onUp = (): void => {
            el.releasePointerCapture(e.pointerId);
            el.removeEventListener("pointermove", onMove);
            el.removeEventListener("pointerup", onUp);
            setResizing(false);
            globalThis.localStorage?.setItem(
                SIDEBAR_WIDTH_KEY,
                String(widthRef.current),
            );
        };
        el.addEventListener("pointermove", onMove);
        el.addEventListener("pointerup", onUp);
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
                // Installed desktop PWA (manifest `display_override:
                // window-controls-overlay`): the browser's title bar is gone and
                // content reaches the window's top edge. macOS overlays its window
                // controls (the traffic lights) top-left — i.e. over this sidebar
                // brand — so inset past them and make the bar a drag handle (no
                // other drag surface exists once the title bar is removed).
                // `env(titlebar-area-x)` is 0 unless WCO is active, so the media
                // query keeps this scoped to the installed PWA.
                "@media (display-mode: window-controls-overlay)": {
                    pl: "calc(env(titlebar-area-x, 0px) + 12px)",
                    WebkitAppRegion: "drag",
                },
            }}
        >
            {/* Brand label intentionally omitted — the empty bar is kept only
                for its two structural roles: matching the right pane's AppBar
                height (so the session list aligns with the chat header) and
                serving as the PWA window-drag region (see sx above). */}
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
                        width: sidebarWidth,
                        flexShrink: 0,
                        borderRight: 1,
                        borderColor: "divider",
                        height: "100%",
                        // Anchor the absolutely-positioned resize handle.
                        position: "relative",
                    }}
                >
                    {sidebarHeader}
                    <Box sx={{ flex: 1, minHeight: 0 }}>{list}</Box>
                    {/* VSCode-style divider: a thin grab strip straddling the
                    right border. Desktop-only — it lives in the non-Drawer
                    branch, so iPad/iPhone never render it. Invisible until
                    hover/drag; an accent line marks it on hover, the full
                    strip tints while dragging. */}
                    <Box
                        role="separator"
                        aria-orientation="vertical"
                        aria-label="Resize sidebar"
                        onPointerDown={startResize}
                        sx={{
                            position: "absolute",
                            top: 0,
                            right: -3,
                            width: 6,
                            height: "100%",
                            cursor: "col-resize",
                            zIndex: 2,
                            // Centered 1px accent line that thickens on hover /
                            // while dragging — the visible part of the handle.
                            "&::after": {
                                content: '""',
                                position: "absolute",
                                top: 0,
                                left: "50%",
                                transform: "translateX(-50%)",
                                width: resizing ? 2 : 1,
                                height: "100%",
                                bgcolor: resizing ? "primary.main" : "transparent",
                                transition: "background-color 120ms",
                            },
                            "&:hover::after": {
                                bgcolor: resizing ? "primary.main" : "primary.light",
                            },
                        }}
                    />
                </Stack>
            )}

            <Stack sx={{ flex: 1, minWidth: 0 }}>
                <AppBar
                    position="static"
                    color="default"
                    elevation={0}
                    // Clear the iPhone status bar / notch: hosted full-bleed in the
                    // atlantis portal iframe, the header would otherwise collide with
                    // the clock. Matches NavShell / the portal chrome.
                    sx={{ pt: "env(safe-area-inset-top, 0px)" }}
                >
                    <Toolbar
                        variant="dense"
                        sx={{
                            borderBottom: 1,
                            borderColor: "divider",
                            // Narrow installed PWA: the sidebar has collapsed to a
                            // drawer, so this bar is full-width and the macOS window
                            // controls now overlay ITS left (the hamburger). Inset
                            // past them only in that mode — when the sidebar is
                            // visible the controls sit over it instead (see
                            // sidebarHeader), and this bar must stay flush.
                            ...(mobile && {
                                "@media (display-mode: window-controls-overlay)": {
                                    pl: "calc(env(titlebar-area-x, 0px) + 12px)",
                                },
                            }),
                        }}
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
                                    fontSize="medium"
                                    sx={{ flexShrink: 0 }}
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
                    </Toolbar>
                </AppBar>

                {active ? (
                    <>
                        <Transcript
                            sessionId={active.id}
                            timeline={timelines.get(active.id) ?? []}
                            status={active.status}
                            provider={active.provider}
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

