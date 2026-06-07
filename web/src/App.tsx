import { useEffect, useRef, useState } from "react";
import {
    Alert,
    AppBar,
    Box,
    Button,
    Chip,
    CircularProgress,
    Divider,
    IconButton,
    List,
    ListItemButton,
    ListItemIcon,
    ListItemText,
    Menu,
    MenuItem,
    Select,
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
    Check as CheckIcon,
    Circle,
    DeleteOutline,
    DragIndicator,
    DriveFileRenameOutline,
    ExpandLess,
    ExpandMore,
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
import { applyUpdate, notify, openSession, reorderSessions, send, useStore } from "./store";
import { useSortable } from "./useSortable";
import { setNotifySetting, useNotifySetting } from "./turnNotify";
import { setVimSetting, useVimSetting } from "./vimSetting";
import {
    FONT_SCALE_PRESETS,
    LINE_HEIGHT_PRESETS,
    nearestPreset,
    PADDING_PRESETS,
    setFontScale,
    setFontVariant,
    setLineHeight,
    setPadding,
    useReadingSettings,
} from "./readingSettings";
import {
    type NavbarPosition,
    setNavbarPosition,
    useNavbarAtBottom,
    useNavbarPosition,
} from "./navbarSettings";
import { FONT_PRESETS, getFontPreset } from "./fonts";
import { ProviderIcon } from "./ProviderIcon";
import { BottomSheet, DetentSheet, ThemeModeControl } from "./_shell";
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
// derivation falls back to the first session AND a one-shot warning snackbar
// fires once the session list loads (see restoredFocusRef / goneCheckedRef).
const ACTIVE_SESSION_KEY = "cowboy:active-session";

function readActiveSession(): string | null {
    return globalThis.localStorage?.getItem(ACTIVE_SESSION_KEY) ?? null;
}

// Status is shown as a single color-coded dot/spinner (no text label), so the
// hue has to carry the whole meaning. The palette tokens are chosen so the
// colors read the same here and in any future status surface:
//   green (success)       — live:     running (idle, ready) or busy (a turn in
//                                      flight). busy renders the green as a spinner.
//   blue  (info)          — starting:  process spinning up, not ready yet (spinner).
//   grey  (text.disabled) — dormant:   exited cleanly + resumable — "asleep, wakes
//                                       on resume". Deliberately NOT a warning hue.
//   amber (warning)       — interrupted: a turn was cut off by a daemon restart —
//                                       unfinished, needs attention. Amber (not the
//                                       crashed red) reads as "incomplete", not "dead".
//   red   (error)         — crashed:   died abnormally, can't reply.
function statusColor(s: Status): string {
    switch (s) {
        case "running":
        case "busy":
            return "success.main";
        case "starting":
            return "info.main";
        case "exited":
            return "text.disabled";
        case "interrupted":
            return "warning.main";
        case "crashed":
            return "error.main";
    }
}

// Human-readable meaning for the dot — surfaced in its tooltip / aria-label,
// since the dot itself is just color. Mirrors the statusColor mapping above.
function statusLabel(s: Status): string {
    switch (s) {
        case "running":
            return "Live";
        case "busy":
            return "Running…";
        case "starting":
            return "Starting…";
        case "exited":
            return "Dormant";
        case "interrupted":
            return "Interrupted";
        case "crashed":
            return "Crashed";
    }
}

// One status indicator, shared by the header and the sidebar list so the
// "green = live/running, grey = dormant" code reads identically everywhere. The
// dot encodes state by color; the tooltip + aria-label spell it out for hover and
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
    // "busy" and "starting" are the *active* states — a turn is in flight, or the
    // process is spinning up — so a static dot would read as idle/stuck. Render
    // them as a tiny spinner sized to the dot (green for busy = "running", blue
    // for starting), with the stroke following statusColor for palette
    // continuity. running / exited / crashed are settled, so they stay a
    // color-coded dot. `color="inherit"` lets the sx `color` (statusColor) drive
    // the stroke instead of a fixed MUI palette slot.
    const active = status === "busy" || status === "starting";
    const indicator = active ? (
            <CircularProgress
                size={11}
                thickness={6}
                color="inherit"
                aria-label={statusLabel(status)}
                sx={[{ flexShrink: 0, color: statusColor(status) }, ...extra]}
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
    // Drag-to-reorder via the leading grip handle (server-authoritative, synced).
    const byId = new Map(sessions.map((s) => [s.id, s]));
    const sortable = useSortable({
        ids: sessions.map((s) => s.id),
        onReorder: reorderSessions,
    });
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
                {sortable.order.map((id) => {
                    const s = byId.get(id);
                    if (!s) return null;
                    return (
                    <ListItemButton
                        key={s.id}
                        ref={sortable.registerItem(s.id)}
                        style={sortable.itemStyle(s.id)}
                        selected={s.id === activeId}
                        onClick={(): void => onPick(s.id)}
                        // Keep the trailing kebab off the screen edge: floor a
                        // right inset so it never hugs the rounded corner / iOS
                        // back-swipe edge (ui.md §7), where it was easy to miss.
                        sx={{ pr: "max(env(safe-area-inset-right), 8px)" }}
                    >
                        {/* Leading grip — drag to reorder. A real 44px IconButton
                            (Apple HIG touch min) with a FIXED 24px glyph: the icon
                            is otherwise rem-based, so the reading font-scale (e.g.
                            85% → root 13.6px) shrank it to ~16px and the affordance
                            read as tiny. Pin it in px so this touch target stays
                            standard-MUI-sized regardless of reading prefs.
                            stopPropagation in handleProps keeps a row tap (select)
                            and the sheet's drag separate from a reorder. */}
                        <IconButton
                            {...sortable.handleProps(s.id)}
                            aria-label="Drag to reorder"
                            sx={{
                                width: 44,
                                height: 44,
                                flexShrink: 0,
                                ml: -1,
                                color: "text.disabled",
                            }}
                        >
                            <DragIndicator sx={{ fontSize: 24 }} />
                        </IconButton>
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
                            // 44px tap target (Apple HIG min) with a FIXED 24px
                            // glyph. The icon is rem-based, so the reading
                            // font-scale (85% → root 13.6px) shrank it to ~19px;
                            // pin it in px so it stays standard-MUI-sized. (Was
                            // size="small" + edge="end" — tiny + edge-flush, mis-
                            // tap-prone on touch.)
                            sx={{ ml: 0.5, width: 44, height: 44, flexShrink: 0 }}
                        >
                            <MoreVert sx={{ fontSize: 24 }} />
                        </IconButton>
                    </ListItemButton>
                    );
                })}
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
    const navbarAtBottom = useNavbarAtBottom();
    const create = (): void => {
        // POST (not the fire-and-forget WS `new_session`) so we get the assigned
        // id back synchronously and can focus the new session the moment it's
        // created.
        void fetch("/api/sessions", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ provider, cwd, origin: "web" }),
        })
            .then((r) => (r.ok ? r.json() : null))
            .then((data: { session_id?: string } | null) => {
                if (data?.session_id) onCreated(data.session_id);
            })
            .catch(() => {
                // Network/daemon error surfaces via the WS error channel.
            });
        onClose();
    };
    // BottomSheet (not a centered Dialog) to match the rest of the modals — they
    // all rise from the bottom on the mobile tier.
    return (
        <BottomSheet
            forceSheet={navbarAtBottom}
            open={open}
            onClose={onClose}
            title="New session"
            actions={
                <>
                    <Button onClick={onClose} color="inherit">
                        Cancel
                    </Button>
                    <Button onClick={create} variant="contained">
                        Create
                    </Button>
                </>
            }
        >
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
                        WORKING_DIRS.find((w) => w.value === cwd)?.help ?? ""
                    }
                >
                    {WORKING_DIRS.map((w) => (
                        <MenuItem key={w.value} value={w.value}>
                            {w.label}
                        </MenuItem>
                    ))}
                </TextField>
            </Stack>
        </BottomSheet>
    );
}

// Delay before the update bar hard-reloads into the new build on its own. No
// visible countdown — a single timer, so the bar doesn't re-render each second.
const UPDATE_RELOAD_MS = 2000;

// Full-width overlay bar that tracks the live WebSocket (see store.ts `Banner`).
// All three states are the SAME bar — `position: fixed` keeps it on top of
// everything and out of the layout flow, so it never pushes the panes down or
// disturbs the current session (`pointer-events: none` also lets clicks fall
// through to the chrome it floats over):
//   - red "down"          — reconnect has failed past the threshold (spinner);
//   - green "reconnected"  — recovery, auto-dismissed (check);
//   - blue "update"        — a redeploy was detected; counts 3→0 and then
//                            hard-reloads into the new build on its own.
function ConnectionBanner(): React.JSX.Element | null {
    const { banner } = useStore();
    const isUpdate = banner?.kind === "update";

    // Auto-reload into the new build after a short fixed delay — one timer, no
    // per-second countdown (so the bar shows a static message and never
    // re-renders while it waits).
    useEffect(() => {
        if (!isUpdate) return undefined;
        const t = setTimeout(applyUpdate, UPDATE_RELOAD_MS);
        return (): void => clearTimeout(t);
    }, [isUpdate]);

    if (!banner) return null;

    const palette =
        banner.kind === "down"
            ? "error"
            : banner.kind === "reconnected"
              ? "success"
              : "info";
    const label =
        banner.kind === "down"
            ? "Connection lost — reconnecting…"
            : banner.kind === "reconnected"
              ? "Reconnected"
              : "New version · reloading…";
    return (
        <Box
            role="status"
            aria-live="polite"
            sx={{
                position: "fixed",
                top: 0,
                left: 0,
                right: 0,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                gap: 1,
                px: 2,
                py: 0.75,
                // Owns the notch when shown (it's the topmost element).
                pt: "calc(env(safe-area-inset-top, 0px) + 6px)",
                bgcolor: `${palette}.main`,
                color: `${palette}.contrastText`,
                fontSize: "0.8125rem",
                fontWeight: 500,
                // Purely informational — never eat clicks meant for the UI
                // underneath it.
                pointerEvents: "none",
                zIndex: (t) => t.zIndex.tooltip + 1,
            }}
        >
            {banner.kind === "down" && (
                <CircularProgress size={14} color="inherit" thickness={5} />
            )}
            {banner.kind === "reconnected" && (
                <CheckIcon sx={{ fontSize: 18 }} />
            )}
            <span>{label}</span>
        </Box>
    );
}

export function App({
    themeMode,
    onSetThemeMode,
}: {
    themeMode: ThemeMode;
    onSetThemeMode: (m: ThemeMode) => void;
}): React.JSX.Element {
    const { sessions, timelines, hydrated, lastError, sessionsLoaded, connected } =
        useStore();
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
    // Navbar placement: when the user picks "bottom" on the compact tier
    // (`< lg`, tablets included) the AppBar moves below the transcript, just
    // above the composer (mobile-browser bottom-bar feel). The modals read the
    // same flag and force their bottom-sheet surface (see BottomSheet
    // `forceSheet`), so a tablet's bottom navbar gets bottom-up modals too
    // rather than centered dialogs.
    const navbarAtBottom = useNavbarAtBottom();
    const [activeId, setActiveId] = useState<string | null>(readActiveSession);
    // The focus id we restored from localStorage at mount (PWA relaunch / reload).
    // Held in a ref so the gone-check fires exactly once after the first session
    // list arrives — if that session was deleted while we were away, the
    // `active` derivation silently falls back to sessions[0], which would hide
    // the loss; this surfaces it as a warning snackbar instead.
    const restoredFocusRef = useRef<string | null>(readActiveSession());
    const goneCheckedRef = useRef(false);
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

    // Fault tolerance for a restored focus that vanished: once the first session
    // list has arrived, if the id we persisted no longer names a live session,
    // warn the user (the view already fell back to sessions[0]). Runs once.
    useEffect(() => {
        if (!sessionsLoaded || goneCheckedRef.current) return;
        goneCheckedRef.current = true;
        const restored = restoredFocusRef.current;
        if (restored && !sessions.some((s) => s.id === restored)) {
            notify(
                "Last session is no longer available — opened another instead.",
                "warning",
            );
        }
    }, [sessionsLoaded, sessions]);

    // Revive-on-open (design §7): tell the daemon which session is focused so it
    // warms that agent — reviving one whose agent died with a daemon restart —
    // before the user types, instead of only on the first prompt. Keyed on the
    // id (not the meta object) so status churn doesn't re-fire it; reconnects
    // are handled by the store re-asserting the open id on every onopen.
    useEffect(() => {
        if (active) openSession(active.id);
    }, [active?.id]);

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
        <Box
            sx={{
                display: "flex",
                flexDirection: "column",
                height: "100%",
                width: "100%",
            }}
        >
            {/* Full-width connection/version banner, above the whole layout so
                it spans both panes and pushes them down when shown. */}
            <ConnectionBanner />
            <Box
                sx={{
                    display: "flex",
                    flex: 1,
                    minHeight: 0,
                    width: "100%",
                }}
            >
            {mobile ? (
                // The shared momentum sheet — same affordance as every other
                // mobile sheet (Settings, etc.). Its anchor FOLLOWS the navbar:
                // when the navbar sits at the bottom the list rises from the
                // bottom too (mobile-browser feel), otherwise it drops from the
                // top. DetentSheet owns the safe-area insets for its anchored
                // edge, so the iPhone/iPad notch + rounded corners are cleared
                // without per-anchor padding here.
                <DetentSheet
                    open={drawerOpen}
                    onClose={(): void => setDrawerOpen(false)}
                    anchor={navbarAtBottom ? "bottom" : "top"}
                    ariaLabel="Sessions"
                    surfaceColor={theme.palette.background.default}
                >
                    {/* Cancel DetentSheet's body side padding (px:2) for the
                        session list so the rows span the sheet's full width —
                        scoped here, NOT in DetentSheet, so every other sheet
                        (Settings, …) keeps its gutter. The desktop sidebar
                        renders `list` directly (no wrapper), so it's unaffected. */}
                    <Box sx={{ mx: -2 }}>{list}</Box>
                </DetentSheet>
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

            <Stack
                sx={{
                    flex: 1,
                    minWidth: 0,
                    // Lift the whole column off the on-screen keyboard + its
                    // iOS-native accessory bar: this padding (the keyboard's
                    // overlap, published by useKeyboardInset) reserves space at
                    // the bottom, so the flex:1 transcript shrinks and the bottom
                    // group (composer, or the navbar in bottom mode) rises clear
                    // of the keyboard. 0 when no keyboard.
                    pb: "var(--kb-inset, 0px)",
                }}
            >
                <AppBar
                    position="static"
                    // `color="transparent"` + an explicit theme surface, NOT
                    // `color="default"`: MUI's "default" AppBar resolves to a
                    // hardcoded grey (grey[100]/grey[900]) that ignores cowboy's
                    // lavender palette — the bar read grey while the iOS status
                    // bar (applyThemeColor → theme-color meta) and the app body
                    // (background.default) were both lavender. Pin it to
                    // `background.default` so status bar → navbar → transcript are
                    // one continuous themed surface in light AND dark.
                    color="transparent"
                    elevation={0}
                    sx={{
                        bgcolor: "background.default",
                        color: "text.primary",
                        // Bottom mode (mobile, navbar-pos=bottom): flex `order` puts
                        // the bar at the VERY bottom — below the transcript (0) AND
                        // the composer (1) — for a mobile-browser bottom-bar feel.
                        // Top mode: order 0, first child.
                        order: navbarAtBottom ? 2 : 0,
                        // Own the safe-area inset of whichever edge the bar hugs.
                        // Top: clear the status bar / notch. Bottom: sit TIGHT into
                        // the home-indicator zone the same way the composer action
                        // row does — `home-inset − 20px` (≈14px on a home-bar iPhone
                        // instead of the full ~34px), floored to 2px on devices
                        // without a home bar — so the bar drops low instead of
                        // floating with a big gap below it. Landscape rounded-corner
                        // side insets (pl/pr) keep the bar clear of the iPhone/iPad
                        // R角. All env() insets are 0 off-device + when hosted.
                        pt: navbarAtBottom ? 0 : "env(safe-area-inset-top, 0px)",
                        pb: navbarAtBottom ? "max(calc(env(safe-area-inset-bottom) - 20px), 2px)" : 0,
                        pl: navbarAtBottom ? "env(safe-area-inset-left, 0px)" : 0,
                        pr: navbarAtBottom ? "env(safe-area-inset-right, 0px)" : 0,
                    }}
                >
                    <Toolbar
                        variant="dense"
                        sx={{
                            // Separator faces the transcript: bottom border at the
                            // top, top border when the bar sits at the bottom.
                            borderBottom: navbarAtBottom ? 0 : 1,
                            borderTop: navbarAtBottom ? 1 : 0,
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
                            // No `edge="start"`: its negative left margin pulled the
                            // hamburger tight to the screen edge, while Settings (no
                            // edge="end") sits at the Toolbar's normal gutter — so the
                            // bar read lopsided. Dropping it gives the hamburger the
                            // same gutter, symmetric with the gear on the right.
                            <IconButton
                                onClick={(): void => setDrawerOpen(true)}
                                // Fixed 40px box on touch (matching the composer
                                // action row's TOOLBAR_ICON_BTN). A default
                                // IconButton sizes to its glyph, which scales with
                                // the global font zoom — so the hamburger drifted
                                // out of line with the slash button below it at
                                // non-100% font sizes. A fixed box keeps both
                                // centered glyphs at the same x at any scale.
                                sx={{ mr: 1, "@media (pointer: coarse)": { width: 40, height: 40 } }}
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
                                    fontSize="small"
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
                                <Tooltip title="Rename session">
                                    <IconButton
                                        size="small"
                                        aria-label="rename session"
                                        onClick={(): void =>
                                            setPendingRename(active)}
                                        sx={{ flexShrink: 0 }}
                                    >
                                        <DriveFileRenameOutline fontSize="small" />
                                    </IconButton>
                                </Tooltip>
                            </Stack>
                        ) : (
                            // No session: the content pane already says "No
                            // session selected", so the bar shows nothing — no
                            // redundant brand/emoji.
                            <Box sx={{ flex: 1, minWidth: 0 }} />
                        )}
                        <IconButton
                            onClick={(): void => setSettingsOpen(true)}
                            aria-label="settings"
                            title="Settings"
                            // Fixed 40px box on touch so the gear stays aligned
                            // with the action row's send/stop button below it at
                            // any font scale (see the hamburger note above).
                            sx={{ "@media (pointer: coarse)": { width: 40, height: 40 } }}
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
                            // Skeleton until this session's history snapshot
                            // lands (vs an empty session, which is hydrated).
                            loading={!hydrated.has(active.id)}
                            // While the WS is down the "working" spinner must not
                            // keep spinning on a stale status (daemon restart).
                            connected={connected}
                        />
                        {/* Bottom mode: order 1 sits the composer above the
                            navbar (order 2, the very bottom) and below the
                            transcript (0). Top mode: order 0 (default DOM order).
                            minWidth:0 so long content can't overflow the column. */}
                        <Box sx={{ order: navbarAtBottom ? 1 : 0, minWidth: 0 }}>
                            <Composer
                                // Remount per session: each session owns its draft
                                // (seeded from the per-session draft store) and a
                                // fresh CodeMirror editor, so one session's
                                // in-progress text never bleeds into another.
                                key={active.id}
                                sessionId={active.id}
                                status={active.status}
                            />
                        </Box>
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
            </Box>

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
                    severity={lastError?.severity ?? "error"}
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
    const notify = useNotifySetting();
    const reading = useReadingSettings();
    // Font picker is collapsed by default (the 7 preview cards otherwise fill the
    // screen); the collapsed summary still shows the current face. Resets to
    // collapsed each time Settings opens — the desired compact default.
    const [fontOpen, setFontOpen] = useState(false);
    const selectedFont = getFontPreset(reading.fontVariant);
    // Shared card style for both the collapsed summary and the expanded list.
    const fontCardSx = (active: boolean): SxProps<Theme> => ({
        cursor: "pointer",
        borderRadius: 1,
        border: 2,
        borderColor: active ? "primary.main" : "divider",
        px: 1.5,
        py: 1,
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 1,
        transition: "border-color 0.15s ease",
        "&:hover": { borderColor: active ? "primary.main" : "text.secondary" },
    });
    const navbarPos = useNavbarPosition();
    const navbarAtBottom = useNavbarAtBottom();
    const theme = useTheme();
    // Navbar position is offered on the whole compact tier (`< lg`, tablets
    // included); desktop is always top.
    const mobile = useMediaQuery(theme.breakpoints.down("lg"));
    // Vim is desktop-only (ComposerEditor won't load it on touch), so the
    // toggle only appears where a physical keyboard exists.
    const desktop = useMediaQuery("(pointer: fine) and (hover: hover)");
    return (
        <BottomSheet open={open} onClose={onClose} title="Settings" forceSheet={navbarAtBottom}>
            <Stack spacing={3}>
                <ThemeModeControl value={themeMode} onChange={onSetThemeMode} />

                {/* Reading comfort — scales only the transcript message content
                    and its side gutter (chrome stays put). Dropdowns, not a
                    slider: they tap cleanly on touch (see readingSettings). */}
                <Divider />
                <Stack spacing={2}>
                    <Typography variant="overline" color="text.secondary">
                        Reading
                    </Typography>

                    {/* Font family — collapsible. Collapsed shows the current
                        face previewed in itself (so the selection is always
                        visible without the 7-card list filling the screen);
                        expanding drops the full picker, and choosing a face
                        auto-collapses. Each card previews its own @fontsource
                        woff2 (loaded lazily once selected). */}
                    {fontOpen ? (
                        <Stack spacing={0.75}>
                            {/* Collapse header */}
                            <Box
                                onClick={(): void => setFontOpen(false)}
                                sx={{
                                    display: "flex",
                                    alignItems: "center",
                                    justifyContent: "space-between",
                                    cursor: "pointer",
                                    px: 0.5,
                                }}
                            >
                                <Typography variant="body2">
                                    Reading font
                                </Typography>
                                <ExpandLess sx={{ color: "text.secondary" }} />
                            </Box>
                            {FONT_PRESETS.map((preset) => {
                                const selected =
                                    reading.fontVariant === preset.id;
                                return (
                                    <Box
                                        key={preset.id}
                                        onClick={(): void => {
                                            setFontVariant(preset.id);
                                            setFontOpen(false);
                                        }}
                                        sx={fontCardSx(selected)}
                                    >
                                        <Box sx={{ minWidth: 0 }}>
                                            <Typography
                                                sx={{
                                                    fontFamily: preset.stack,
                                                    fontSize: "1.05rem",
                                                    lineHeight: 1.3,
                                                }}
                                                noWrap
                                            >
                                                {preset.label} · 阅读 Aa
                                            </Typography>
                                            <Typography
                                                variant="caption"
                                                color="text.secondary"
                                                noWrap
                                            >
                                                {preset.note}
                                            </Typography>
                                        </Box>
                                        {selected && (
                                            <CheckIcon
                                                fontSize="medium"
                                                color="primary"
                                            />
                                        )}
                                    </Box>
                                );
                            })}
                        </Stack>
                    ) : (
                        // Collapsed summary — the current face, tap to change.
                        <Box
                            onClick={(): void => setFontOpen(true)}
                            aria-label="Change reading font"
                            sx={fontCardSx(false)}
                        >
                            <Box sx={{ minWidth: 0 }}>
                                <Typography
                                    variant="caption"
                                    color="text.secondary"
                                >
                                    Reading font
                                </Typography>
                                <Typography
                                    sx={{
                                        fontFamily: selectedFont.stack,
                                        fontSize: "1.05rem",
                                        lineHeight: 1.3,
                                    }}
                                    noWrap
                                >
                                    {selectedFont.label} · 阅读 Aa
                                </Typography>
                            </Box>
                            <ExpandMore sx={{ color: "text.secondary" }} />
                        </Box>
                    )}

                    <Stack
                        direction="row"
                        alignItems="center"
                        justifyContent="space-between"
                        spacing={2}
                    >
                        <Stack>
                            <Typography variant="body2">Font size</Typography>
                            <Typography
                                variant="caption"
                                color="text.secondary"
                            >
                                Scales all app text
                            </Typography>
                        </Stack>
                        <Select
                            size="small"
                            value={nearestPreset(
                                reading.fontScale,
                                FONT_SCALE_PRESETS,
                            )}
                            onChange={(e): void =>
                                setFontScale(Number(e.target.value))}
                            sx={{ minWidth: 104 }}
                        >
                            {FONT_SCALE_PRESETS.map((v) => (
                                <MenuItem key={v} value={v}>
                                    {Math.round(v * 100)}%
                                </MenuItem>
                            ))}
                        </Select>
                    </Stack>
                    <Stack
                        direction="row"
                        alignItems="center"
                        justifyContent="space-between"
                        spacing={2}
                    >
                        <Stack>
                            <Typography variant="body2">Padding</Typography>
                            <Typography
                                variant="caption"
                                color="text.secondary"
                            >
                                Side gutter of the transcript + composer
                            </Typography>
                        </Stack>
                        <Select
                            size="small"
                            value={nearestPreset(
                                reading.padding,
                                PADDING_PRESETS,
                            )}
                            onChange={(e): void =>
                                setPadding(Number(e.target.value))}
                            sx={{ minWidth: 104 }}
                        >
                            {PADDING_PRESETS.map((v) => (
                                <MenuItem key={v} value={v}>
                                    {`${v}px`}
                                </MenuItem>
                            ))}
                        </Select>
                    </Stack>
                    <Stack
                        direction="row"
                        alignItems="center"
                        justifyContent="space-between"
                        spacing={2}
                    >
                        <Stack>
                            <Typography variant="body2">Line height</Typography>
                            <Typography
                                variant="caption"
                                color="text.secondary"
                            >
                                Spacing between lines of text
                            </Typography>
                        </Stack>
                        <Select
                            size="small"
                            value={nearestPreset(
                                reading.lineHeight,
                                LINE_HEIGHT_PRESETS,
                            )}
                            onChange={(e): void =>
                                setLineHeight(Number(e.target.value))}
                            sx={{ minWidth: 104 }}
                        >
                            {LINE_HEIGHT_PRESETS.map((v) => (
                                <MenuItem key={v} value={v}>
                                    {v.toFixed(1)}
                                </MenuItem>
                            ))}
                        </Select>
                    </Stack>
                </Stack>
                <Divider />
                <Stack
                    direction="row"
                    alignItems="center"
                    justifyContent="space-between"
                    spacing={2}
                >
                    <Stack>
                        <Typography variant="body2">Turn-complete alert</Typography>
                        <Typography variant="caption" color="text.secondary">
                            Sound + vibration when an agent finishes
                        </Typography>
                    </Stack>
                    <Switch
                        checked={notify}
                        onChange={(e): void => setNotifySetting(e.target.checked)}
                        inputProps={{ "aria-label": "Turn-complete alert" }}
                    />
                </Stack>
                {mobile && (
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
                                    Navbar position
                                </Typography>
                                <Typography
                                    variant="caption"
                                    color="text.secondary"
                                >
                                    Bottom = mobile-browser style
                                </Typography>
                            </Stack>
                            <Select
                                size="small"
                                value={navbarPos}
                                onChange={(e): void =>
                                    setNavbarPosition(
                                        e.target.value as NavbarPosition,
                                    )}
                                sx={{ minWidth: 104 }}
                            >
                                <MenuItem value="top">Top</MenuItem>
                                <MenuItem value="bottom">Bottom</MenuItem>
                            </Select>
                        </Stack>
                    </>
                )}
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
    // Hook before the early return (rules of hooks).
    const navbarAtBottom = useNavbarAtBottom();
    if (!session) return null;
    const surface = originLabel(session.origin);
    return (
        <BottomSheet
            forceSheet={navbarAtBottom}
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
    const inputRef = useRef<HTMLInputElement>(null);
    const navbarAtBottom = useNavbarAtBottom();
    useEffect(() => {
        if (!session) return undefined;
        setValue(session.title);
        // Select-all on open so the first keystroke replaces the whole title
        // (you're almost always retyping, not appending). rAF so it runs after
        // the new value is painted into the input.
        const raf = requestAnimationFrame(() => inputRef.current?.select());
        return () => cancelAnimationFrame(raf);
    }, [session?.id, session?.title]);
    if (!session) return null;
    const trimmed = value.trim();
    const canSave = trimmed.length > 0 && trimmed !== session.title;
    const submit = (): void => {
        if (canSave) onConfirm(trimmed);
    };
    return (
        <BottomSheet
            forceSheet={navbarAtBottom}
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
                inputRef={inputRef}
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

