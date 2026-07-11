import { useCallback, useEffect, useRef, useState } from "react";
import {
    Alert,
    alpha,
    AppBar,
    Box,
    Button,
    ButtonBase,
    Chip,
    CircularProgress,
    Divider,
    IconButton,
    List,
    ListItemButton,
    ListItemIcon,
    ListItemText,
    ListSubheader,
    Menu,
    MenuItem,
    Select,
    Skeleton,
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
    Bolt,
    Check as CheckIcon,
    Circle,
    Close as CloseIcon,
    DeleteOutline,
    DragIndicator,
    DriveFileRenameOutline,
    ExpandLess,
    ExpandMore,
    InfoOutlined,
    Menu as MenuIcon,
    MoreVert,
    Schedule,
    Settings as SettingsIcon,
} from "@mui/icons-material";
import { AutoScrollAndStop, Composer, SessionControls } from "./Composer";
import { useTouchComposer } from "./ComposerTextarea";
import { useVimMode, VIM_MODE_COLOR } from "./vimModeStore";
import { claimKeyboard } from "./keyboardClaim";
import { Transcript } from "./Transcript";
import {
    originLabel,
    PROVIDERS,
    type SessionMeta,
    type SessionOrigin,
    type Status,
} from "./protocol";
import {
    AUTO_RESUME_DEFAULT_KEY,
    AUTO_RESUME_TEMPLATE_KEY,
    conn,
    DEFAULT_CONTINUATION_TEMPLATE,
    markSessionHydrated,
    notify,
    openSession,
    renameSession,
    reorderSessions,
    send,
    setSessionAutoResume,
    setSetting,
    useStoreSelector,
} from "./store";
import { useSortable } from "./useSortable";
import { setNotifySetting, setVibrateSetting, useNotifySetting, useVibrateSetting } from "./turnNotify";
import {
    clampComposerColWidth,
    composerColWidthStore,
    setDesktopLayout,
    useDesktopLayout,
} from "./desktopLayout";
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
import { useNavbarAtBottom } from "./navbarSettings";
import { FONT_PRESETS, getFontPreset } from "./fonts";
import { ProviderIcon } from "./ProviderIcon";
import { ConnectionBanner, DetentSheet, ThemeModeControl, useAnyDetentSheetOpen } from "./_shell";
import { Sheet } from "./Sheet";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL } from "./platform";
import { InfoContent } from "./InfoSheet";
import { SegmentedPill } from "./SegmentedPill";
import { fireLabel, fireRel } from "./scheduleTime";
import { ConfirmSendModal } from "./ConfirmSendModal";
import { ResourceLightbox } from "./ResourceLightbox";
import { JudgeInspectorHost } from "./JudgeInspector";
import type { Mode as ThemeMode } from "./theme";
import { persisted } from "./_store/mod.ts";

// Desktop sidebar width: a user-draggable pixel width (VSCode-style divider),
// persisted in localStorage. The bounds keep both panes usable — 240px floor
// (a list row stays readable), 480px ceiling (wider and the list looks empty
// next to the transcript). 300px default sits inside the old fluid
// `clamp(240px, 22vw, 360px)` it replaces. Resize is desktop-only: below the
// `lg` breakpoint the sidebar is a full-width top Drawer (touch layout), which
// has no divider — so neither bound nor handle applies there.
// App-wide bottom status bar (Zed / VSCode style): a thin strip at the very
// bottom of the window. DESKTOP ONLY + only when vim is on (its sole status today
// is the vim mode); a flex row so line:col / language / diagnostics can join it
// later. Reads the live mode from vimModeStore, which ComposerEditor writes. Lives
// inside the composer's measured wrapper so the transcript's `--composer-h`
// reservation includes it automatically.
// One segment of the status bar (VSCode-style). A plain colored label by default;
// pass `onClick` to make it an interactive segment — a ButtonBase, so it gets the
// material hover/ripple AND is picked up by the global haptic delegation for free.
// `icon` slots a small leading glyph. This is the reusable unit the bar is built
// from as more states land.
function StatusItem({
    label,
    color,
    icon,
    tooltip,
    onClick,
    mono = false,
}: {
    label: string;
    color?: string;
    icon?: React.ReactNode;
    tooltip?: string;
    onClick?: () => void;
    mono?: boolean;
}): React.JSX.Element {
    const sx: SxProps<Theme> = {
        display: "inline-flex",
        alignItems: "center",
        gap: 0.5,
        px: 1,
        height: "100%",
        fontSize: "0.6875rem",
        fontWeight: 600,
        letterSpacing: "0.04em",
        lineHeight: 1,
        whiteSpace: "nowrap",
        color: color ?? "text.secondary",
        ...(mono && { fontFamily: "monospace" }),
        ...(onClick && {
            transition: "background-color .12s, color .12s",
            "&:hover": { bgcolor: "action.hover", color: "text.primary" },
        }),
    };
    const inner = (
        <>
            {icon}
            <Box component="span">{label}</Box>
        </>
    );
    const el = onClick
        ? (
            <ButtonBase sx={sx} onClick={onClick}>
                {inner}
            </ButtonBase>
        )
        : <Box sx={sx}>{inner}</Box>;
    return tooltip ? <Tooltip title={tooltip}>{el}</Tooltip> : el;
}

// The app's bottom status bar (Zed/VSCode-style). A full-width strip with a LEFT
// and a RIGHT item group and a flexible spacer between — built from StatusItems so
// new states (session/agent status, token counts, connection, cwd, …) drop into
// `left`/`right` with no layout work. Desktop-only (inside the composer's measured
// wrapper, so the transcript's --composer-h reservation includes it). Renders
// nothing until at least one item exists.
function AppStatusBar({
    sessionId,
    status,
}: {
    sessionId?: string;
    status?: Status;
}): React.JSX.Element | null {
    const vim = useVimSetting();
    const vimMode = useVimMode();
    const touchInput = useTouchComposer();
    // Desktop-only (mobile has the navbar at the bottom; this footer is a
    // Zed/VSCode-style status strip). On desktop it always renders — the session's
    // live controls (auto-scroll + Stop) live here now, so it's never empty.
    if (touchInput) return null;

    const left: React.ReactNode[] = [];
    if (vim) {
        left.push(
            <StatusItem
                key="vim"
                label={vimMode.toUpperCase()}
                color={VIM_MODE_COLOR[vimMode] ?? "text.secondary"}
                tooltip={`Vim — ${vimMode} mode`}
                mono
            />,
        );
    }

    return (
        <Box
            sx={{
                display: "flex",
                alignItems: "center",
                minHeight: 34,
                px: 0.25,
                borderTop: 1,
                borderColor: "divider",
                // A whisper of fill so the bar reads as its own surface (material)
                // without fighting the frosted slab the composer floats on.
                bgcolor: (t) => alpha(t.palette.text.primary, 0.025),
                color: "text.secondary",
                userSelect: "none",
            }}
        >
            <Stack direction="row" alignItems="center">
                {left}
            </Stack>
            <Box sx={{ flex: 1 }} />
            {/* The session's live controls — auto-scroll follow + Stop — moved off
                the navbar into the status bar on desktop (Zed/VSCode keep run/stop in
                the bottom bar). `dense` shrinks them to fit the strip. */}
            {sessionId !== undefined && status !== undefined && (
                <AutoScrollAndStop sessionId={sessionId} status={status} dense />
            )}
        </Box>
    );
}

const SIDEBAR_MIN = 240;
const SIDEBAR_MAX = 480;
const SIDEBAR_DEFAULT = 300;

function clampSidebarWidth(px: number): number {
    return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, px));
}

// Persisted (per-device) sidebar width — seeded into local state for the live
// drag, written back on settle. Re-clamped on read in case the bounds changed
// since it was stored; default when unset/unparseable. String(px) format kept.
const sidebarWidthStore = persisted("cowboy:sidebar-width", SIDEBAR_DEFAULT, {
    serialize: String,
    deserialize: (raw) => {
        const n = Number.parseInt(raw, 10);
        return Number.isFinite(n) ? clampSidebarWidth(n) : SIDEBAR_DEFAULT;
    },
});

// The session the user last had focused, so a page reload (or PWA relaunch)
// reopens it instead of snapping back to the top of the list. Just an id; if it
// names a session that no longer exists (deleted elsewhere) the `active`
// derivation falls back to the first session AND a one-shot warning snackbar
// fires once the session list loads (see restoredFocusRef / goneCheckedRef).
// The session the user last had focused — persisted (per-device) so a reload /
// PWA relaunch reopens it. Just an id (or null).
const activeSessionStore = persisted<string | null>("cowboy:active-session", null, {
    serialize: (id) => id ?? "",
    deserialize: (raw) => (raw === "" ? null : raw),
});

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
    // process is spinning up — so a static dot would read as idle/stuck. Render the
    // active states as a tiny spinner sized to the dot, with the stroke following
    // statusColor for palette continuity. running / exited / crashed are settled,
    // so they stay a color-coded dot. `color="inherit"` lets the sx `color`
    // (statusColor) drive the stroke instead of a fixed MUI palette slot.
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

// Auto-resume indicator (tasks/active/session-auto-resume). Shown whenever the
// session's EFFECTIVE auto-resume is on — override if set, else the global
// default — so turning the global default on marks every inherited session too.
// Effective-off (incl. an explicit opt-out) shows nothing. While a session is
// actually mid-resume (Interrupted + on), a spinner replaces the glyph. Orthogonal
// to StatusDot (what the session IS now); this says what happens after a restart.
function AutoResumeBadge({
    meta,
    defaultOn,
}: {
    meta: SessionMeta;
    defaultOn: boolean;
}): React.JSX.Element | null {
    const effective = meta.auto_resume ?? defaultOn;
    if (!effective) return null;
    if (meta.status === "interrupted") {
        return (
            <Tooltip title="Auto-resume: continuing the interrupted turn…" enterDelay={300}>
                <CircularProgress
                    size={13}
                    thickness={6}
                    sx={{ flexShrink: 0, color: "warning.main" }}
                />
            </Tooltip>
        );
    }
    return (
        <Tooltip title="Auto-resume on" enterDelay={300}>
            <Bolt sx={{ fontSize: 16, flexShrink: 0, color: "info.main" }} />
        </Tooltip>
    );
}

// Scheduled-draft indicator: shown when a session has ≥1 draft with a future
// fire time (`next_schedule_ms` = the soonest). A calm clock glyph in info blue
// (a pending schedule is a notice, not a status/failure — conventions/ui.md §4),
// orthogonal to StatusDot, mirroring AutoResumeBadge. Tooltip spells out when.
function ScheduleBadge({ meta }: { meta: SessionMeta }): React.JSX.Element | null {
    const ms = meta.next_schedule_ms;
    if (ms === undefined) return null;
    return (
        <Tooltip title={`定时发送 · ${fireLabel(ms)}（${fireRel(ms)}）`} enterDelay={300}>
            <Schedule sx={{ fontSize: 15, flexShrink: 0, color: "info.main" }} />
        </Tooltip>
    );
}

function originColor(
    o: SessionOrigin | undefined,
): "primary" | "secondary" | "default" {
    switch (o ?? "api") {
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
    onRequestInfo,
    onRequestRename,
    autoResumeDefault,
    loaded,
}: {
    sessions: SessionMeta[];
    activeId: string | null;
    onPick: (id: string) => void;
    onNew: () => void;
    onRequestDelete: (s: SessionMeta) => void;
    onRequestInfo: (s: SessionMeta) => void;
    onRequestRename: (s: SessionMeta) => void;
    autoResumeDefault: boolean;
    // True once the first session list has arrived over the WS. Until then the
    // list is genuinely UNKNOWN (not "empty") — distinguishing the two avoids the
    // false "No sessions yet." flash on a reload before the snapshot lands.
    loaded: boolean;
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
    const listRef = useRef<HTMLUListElement>(null);
    const sortable = useSortable({
        ids: sessions.map((s) => s.id),
        onReorder: reorderSessions,
        scrollContainer: () => listRef.current,
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
            <List dense ref={listRef} sx={{ flex: 1, overflowY: "auto" }}>
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
                        // Symmetric side gutters so the leading grip + trailing
                        // kebab circles never hug / get clipped by the screen edge
                        // (floored at 12px, but yielding to a larger safe-area
                        // inset on the notch side in landscape — ui.md §7).
                        sx={{
                            pl: "max(env(safe-area-inset-left), 12px)",
                            pr: "max(env(safe-area-inset-right), 12px)",
                        }}
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
                                // No negative margin — the row's pl (12px) is the
                                // gutter; a pull-left would re-clip the circle at
                                // the screen edge.
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
                                        label={s.system ? "System" : originLabel(s.origin)}
                                        color={s.system ? "secondary" : originColor(s.origin)}
                                        sx={{
                                            height: 16,
                                            fontSize: 10,
                                            "& .MuiChip-label": { px: 0.75 },
                                        }}
                                    />
                                    <AutoResumeBadge meta={s} defaultOn={autoResumeDefault} />
                                    <ScheduleBadge meta={s} />
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
                    loaded
                        ? (
                            <Typography
                                variant="body2"
                                color="text.secondary"
                                sx={{ p: 2, textAlign: "center" }}
                            >
                                No sessions yet.
                            </Typography>
                        )
                        : <LoadingState compact />
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
                        if (menuAnchor) onRequestInfo(menuAnchor.row);
                        setMenuAnchor(null);
                    }}
                >
                    <ListItemIcon>
                        <InfoOutlined fontSize="medium" />
                    </ListItemIcon>
                    <ListItemText primary="Info" />
                </MenuItem>
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
                <Divider />
                <ListSubheader sx={{ lineHeight: "32px", bgcolor: "transparent" }}>
                    Auto-resume
                </ListSubheader>
                {(
                    [
                        { v: null, label: `Default (${autoResumeDefault ? "on" : "off"})` },
                        { v: true, label: "On" },
                        { v: false, label: "Off" },
                    ] as const
                ).map((opt) => {
                    const current = (menuAnchor?.row.auto_resume ?? null) === opt.v;
                    return (
                        <MenuItem
                            key={String(opt.v)}
                            selected={current}
                            onClick={(): void => {
                                if (menuAnchor) setSessionAutoResume(menuAnchor.row.id, opt.v);
                                setMenuAnchor(null);
                            }}
                        >
                            <ListItemIcon>
                                {current ? <CheckIcon fontSize="medium" /> : null}
                            </ListItemIcon>
                            <ListItemText primary={opt.label} />
                        </MenuItem>
                    );
                })}
                <Divider />
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
    // Working-dir choices: start from the hard-coded fallback, then replace with
    // the daemon's `/api/workspaces` (host roots + every columbus-managed
    // project) once the dialog opens. Falling back keeps the dialog usable if
    // the endpoint is unreachable (older daemon / fetch error).
    const [workspaces, setWorkspaces] =
        useState<readonly { value: string; label: string; help: string }[]>(WORKING_DIRS);
    // Editable session title. Empty on Create → renameSession no-ops → the
    // daemon's default + first-prompt auto-title apply. RESET to a fresh default
    // on every open (below): the sheet stays mounted, so this state would
    // otherwise survive and show the last-typed value.
    const [title, setTitle] = useState<string>("");
    const titleRef = useRef<HTMLInputElement>(null);
    // Session count, captured in a ref so its default name is computed at open
    // time WITHOUT a session arriving mid-edit clobbering what you're typing.
    const sessions = useStoreSelector((snapshot) => snapshot.sessions);
    const sessionCountRef = useRef(sessions.length);
    sessionCountRef.current = sessions.length;
    // On open: reset to a fresh "New session N" default (N keeps it distinct if
    // you open several without renaming), then focus + select it so you can type
    // a name straight away — or clear it to let the first message auto-name.
    // autoFocus (below) is the keyboard's best shot within the opening tap's
    // gesture window (iOS only raises it for an in-gesture focus); the delayed
    // select() highlights the default once the sheet has mounted the field.
    useEffect(() => {
        if (!open) return undefined;
        setTitle(`New session ${sessionCountRef.current + 1}`);
        const t = globalThis.setTimeout(() => {
            titleRef.current?.focus();
            titleRef.current?.select();
        }, 60);
        return () => globalThis.clearTimeout(t);
    }, [open]);
    useEffect(() => {
        if (!open) return;
        void fetch("/api/workspaces")
            .then((r) => (r.ok ? r.json() : null))
            .then((data: { value: string; label: string; help: string }[] | null) => {
                if (Array.isArray(data) && data.length > 0) setWorkspaces(data);
            })
            .catch(() => {
                // Keep the hard-coded fallback on any error.
            });
    }, [open]);
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
                if (data?.session_id) {
                    // Known-empty: no history is coming (the agent only starts on the
                    // first prompt, and OpenSession doesn't snapshot), so mark it
                    // hydrated now — otherwise the transcript skeleton spins forever.
                    markSessionHydrated(data.session_id);
                    // Apply the title set in the modal. renameSession trims +
                    // no-ops on empty, so a cleared title falls back to the
                    // daemon default + first-prompt auto-title.
                    renameSession(data.session_id, title);
                    onCreated(data.session_id);
                }
            })
            .catch(() => {
                // Network/daemon error surfaces via the WS error channel.
            });
        onClose();
    };
    // BottomSheet (not a centered Dialog) to match the rest of the modals — they
    // all rise from the bottom on the mobile tier.
    return (
        <Sheet
            forceSheet={navbarAtBottom}
            open={open}
            onClose={onClose}
            title="New session"
            actions={
                <>
                    <Button onClick={onClose} color="inherit">
                        Cancel
                        <Kbd keys="Esc" />
                    </Button>
                    <Button onClick={create} variant="contained">
                        Create
                        <Kbd keys={ENTER_LABEL} />
                    </Button>
                </>
            }
        >
            <Stack spacing={2} sx={{ mt: 1 }}>
                <TextField
                    label="Title"
                    value={title}
                    onChange={(e): void => setTitle(e.target.value)}
                    inputRef={titleRef}
                    autoFocus
                    onFocus={(e): void => {
                        // Select the whole default ("New session N") on EVERY focus, so
                        // tapping the field replaces it in one go. Deferred a frame — iOS
                        // collapses a synchronous select() back to a caret. Same logic as
                        // the session-rename field (Composer.tsx).
                        const input = e.target as HTMLInputElement;
                        requestAnimationFrame(() => input.select());
                    }}
                    onKeyDown={(e): void => {
                        // Enter from the title field confirms (matches the ⏎ keycap on
                        // Create). Field-level, NOT a global useConfirmEnter: this modal
                        // has Provider/Working-dir Selects, and a global capture handler
                        // would hijack the Enter that picks an open dropdown option.
                        if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                            e.preventDefault();
                            create();
                        }
                    }}
                    placeholder="Name this session"
                    helperText="Clear to auto-name from the first message"
                />
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
                        workspaces.find((w) => w.value === cwd)?.help ?? ""
                    }
                >
                    {workspaces.map((w) => (
                        <MenuItem key={w.value} value={w.value}>
                            {w.label}
                        </MenuItem>
                    ))}
                </TextField>
            </Stack>
        </Sheet>
    );
}

export function App({
    themeMode,
    onSetThemeMode,
}: {
    themeMode: ThemeMode;
    onSetThemeMode: (m: ThemeMode) => void;
}): React.JSX.Element {
    const sessions = useStoreSelector((snapshot) => snapshot.sessions);
    const timelines = useStoreSelector((snapshot) => snapshot.timelines);
    const hydrated = useStoreSelector((snapshot) => snapshot.hydrated);
    const lastError = useStoreSelector((snapshot) => snapshot.lastError);
    const sessionsLoaded = useStoreSelector((snapshot) => snapshot.sessionsLoaded);
    const connected = useStoreSelector((snapshot) => snapshot.connected);
    const settings = useStoreSelector((snapshot) => snapshot.settings);
    const autoResumeDefaultOn = settings[AUTO_RESUME_DEFAULT_KEY] === true;
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
    // A full-screen frosted COVER sheet (compose/edit/settings) bleeds the app's
    // own frosted navbar/composer chrome through it as a bright blob (double
    // frosting). Fade that chrome out while ANY sheet is open so the cover shows
    // only the uniformly-dimmed transcript — consistent top color, like Settings.
    const anySheetOpen = useAnyDetentSheetOpen();
    // Floating-glass bottom (bottom-navbar mode): the composer + navbar float as
    // frosted overlays over a full-height transcript. Publish their measured
    // heights as CSS vars on the column so the transcript reserves that space
    // (its `bottomInset`) — content scrolls UNDER the glass but the newest message
    // clears it. Re-measures on drafts/queue expand + keyboard, so the scroll
    // RANGE tracks the panel with NO column reflow (column-reverse keeps the
    // newest pinned just above the growing panel).
    const columnRef = useRef<HTMLDivElement>(null);
    const [activeId, setActiveId] = useState<string | null>(activeSessionStore.get);
    // Floating-glass inset: publish the panel's TRUE live height — the AppBar plus
    // the composer (the latter INCLUDING an expanded queue/drafts panel) — as CSS
    // vars on the column so the transcript reserves exactly that much bottom space
    // (its `bottomInset`) AND the single frosted slab behind the panel is sized to
    // it; keep both accurate as the panel grows/shrinks.
    //
    // Driven by CALLBACK refs + ONE persistent ResizeObserver, NOT a mount-time
    // effect. Why: the composer mounts only once the session list arrives and
    // `active` flips non-null, which happens WITHOUT activeId changing (activeId is
    // restored from localStorage at first render). A `[navbarAtBottom, activeId]`-
    // keyed effect therefore ran while the composer was still unmounted — wrote
    // --composer-h: 0px, observed a null element, and never re-fired — so the panel
    // permanently covered the newest messages (the "看不到 / 没有动态适应" bug).
    // Callback refs observe each element the instant it actually mounts and
    // re-measure, so the reservation is right from first paint and tracks resizes.
    //
    // Deliberately NOT gated on scroll/sticky state: the reservation must stay
    // accurate at all times so the newest message can never hide behind the glass.
    // Reserving the space is orthogonal to auto-scroll (that stays the Transcript's
    // FOLLOW decision); column-reverse keeps the newest pinned just above the panel,
    // so an accurate inset just lifts it clear with no content reflow.
    const roRef = useRef<ResizeObserver | null>(null);
    const appBarElRef = useRef<HTMLElement | null>(null);
    const composerElRef = useRef<HTMLElement | null>(null);
    const measureGlass = useCallback((): void => {
        const col = columnRef.current;
        if (!col) return;
        col.style.setProperty("--navbar-h", `${appBarElRef.current?.offsetHeight ?? 0}px`);
        col.style.setProperty("--composer-h", `${composerElRef.current?.offsetHeight ?? 0}px`);
    }, []);
    const observeGlass = useCallback(
        (slot: "appbar" | "composer", el: HTMLElement | null): void => {
            roRef.current ??= new ResizeObserver((): void => measureGlass());
            const ro = roRef.current;
            const prev = slot === "appbar" ? appBarElRef.current : composerElRef.current;
            if (prev) ro.unobserve(prev);
            if (slot === "appbar") appBarElRef.current = el;
            else composerElRef.current = el;
            if (el) ro.observe(el);
            measureGlass();
        },
        [measureGlass],
    );
    const appBarRef = useCallback(
        (el: HTMLDivElement | null): void => observeGlass("appbar", el),
        [observeGlass],
    );
    const composerRef = useCallback(
        (el: HTMLDivElement | null): void => observeGlass("composer", el),
        [observeGlass],
    );
    // Disconnect the shared observer on unmount.
    useEffect(() => (): void => roRef.current?.disconnect(), []);
    // Whether the transcript actually overflows (has content scrolling under the
    // floating composer glass). Gates the composer slab's up-shadow: an
    // empty/short conversation has nothing beneath the glass, so the "floating
    // above the scroll" shadow would be a lie — the Transcript reports this.
    const [transcriptScrollable, setTranscriptScrollable] = useState(false);
    // The focus id we restored from localStorage at mount (PWA relaunch / reload).
    // Held in a ref so the gone-check fires exactly once after the first session
    // list arrives — if that session was deleted while we were away, the
    // `active` derivation silently falls back to sessions[0], which would hide
    // the loss; this surfaces it as a warning snackbar instead.
    const restoredFocusRef = useRef<string | null>(activeSessionStore.get());
    const goneCheckedRef = useRef(false);
    const [drawerOpen, setDrawerOpen] = useState(false);
    const [dialogOpen, setDialogOpen] = useState(false);
    const [settingsOpen, setSettingsOpen] = useState(false);
    // Settings + Info are one merged sheet; this picks which tab it opens on.
    const [settingsTab, setSettingsTab] = useState<"settings" | "info">("settings");
    const openSettings = (tab: "settings" | "info"): void => {
        setSettingsTab(tab);
        setSettingsOpen(true);
    };
    // (The no-judge-key warning now lives in the unified TurnStatusOverlay — the
    // blue "no key" pill — so it's no longer a separate top-of-content Notice.)
    // Desktop sidebar width + live-drag flag. Width is a pixel value (not the
    // old fluid clamp) so the divider can set it directly; `resizing` drives
    // the handle's active highlight and a body-wide drag cursor / no-select.
    const [sidebarWidth, setSidebarWidth] = useState<number>(sidebarWidthStore.get);
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
    // Two-column (Zed-style) split layout — desktop-only, opt-in. Active only when
    // the setting is "split" AND this is a real desktop: a fine pointer (not a
    // touch tablet) and ≥lg so the session list is a column (not a Drawer) — i.e.
    // there's room for THREE columns (sessions | composer | chat). Otherwise the
    // single-column overlay renders and the setting is a no-op (mobile unaffected).
    const splitLayout = useDesktopLayout();
    const pointerFine = useMediaQuery("(pointer: fine) and (hover: hover)");
    const splitActive = splitLayout === "split" && !mobile && pointerFine;
    // Composer-column width + live-drag, mirroring the sidebar splitter: a local
    // value during the drag (persisted on release, not per-pixel) backed by the
    // global composer-col-width store. `colResizing` drives the body drag cursor.
    const [colWidth, setColWidth] = useState<number>(composerColWidthStore.get);
    const [colResizing, setColResizing] = useState(false);
    const colWidthRef = useRef(colWidth);
    colWidthRef.current = colWidth;
    useEffect(() => {
        if (!colResizing) return undefined;
        const { body } = document;
        const prevCursor = body.style.cursor;
        const prevSelect = body.style.userSelect;
        body.style.cursor = "col-resize";
        body.style.userSelect = "none";
        return (): void => {
            body.style.cursor = prevCursor;
            body.style.userSelect = prevSelect;
        };
    }, [colResizing]);
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
    // Per-session info dialog target (kebab → Info).
    const [pendingInfo, setPendingInfo] = useState<SessionMeta | null>(null);
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
            activeSessionStore.set(active.id);
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

    // Self-heal the "stuck on the loading skeleton" case (a fresh load that raced
    // a daemon restart — the deploy window: SW reloads the tab while cowboy is
    // briefly down, so the first connect fails). If the first session list hasn't
    // arrived after a grace, reload ONCE — by then the daemon is back, so the
    // reload connects cleanly. A per-tab flag guards against a loop when the daemon
    // is genuinely down: the SECOND stall doesn't auto-reload (LoadingState's own
    // 8s "reload" button takes over). Cleared the moment sessions load.
    useEffect(() => {
        const KEY = "cowboy:stall-reloaded";
        if (sessionsLoaded) {
            globalThis.sessionStorage.removeItem(KEY);
            return undefined;
        }
        const t = globalThis.setTimeout(() => {
            if (!globalThis.sessionStorage.getItem(KEY)) {
                globalThis.sessionStorage.setItem(KEY, "1");
                globalThis.location.reload();
            }
        }, 7000);
        return () => globalThis.clearTimeout(t);
    }, [sessionsLoaded]);

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
            sidebarWidthStore.set(widthRef.current);
        };
        el.addEventListener("pointermove", onMove);
        el.addEventListener("pointerup", onUp);
    }

    // Splitter between the composer column (left) and the transcript (right) in
    // split mode. Same mechanics as startResize: pointer-capture so a fast drag
    // never drops, clamp each move, persist once on release. Drag right → the
    // composer column grows.
    function startColResize(e: React.PointerEvent<HTMLDivElement>): void {
        if (e.button !== 0) return;
        e.preventDefault();
        const startX = e.clientX;
        const startWidth = colWidthRef.current;
        const el = e.currentTarget;
        el.setPointerCapture(e.pointerId);
        setColResizing(true);
        const onMove = (ev: PointerEvent): void => {
            setColWidth(clampComposerColWidth(startWidth + (ev.clientX - startX)));
        };
        const onUp = (): void => {
            el.releasePointerCapture(e.pointerId);
            el.removeEventListener("pointermove", onMove);
            el.removeEventListener("pointerup", onUp);
            setColResizing(false);
            composerColWidthStore.set(colWidthRef.current);
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
            onRequestInfo={(s): void => setPendingInfo(s)}
            onRequestRename={(s): void => {
                claimKeyboard(); // raise the keyboard in-gesture (iOS)
                setPendingRename(s);
            }}
            autoResumeDefault={autoResumeDefaultOn}
            loaded={sessionsLoaded}
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
                // Match the right pane's AppBar Toolbar EXACTLY (it overrides the
                // dense 48px down to 44), so the sidebar header's bottom border lines
                // up with the chat header's edge instead of sitting 4px lower.
                minHeight: 44,
                "@media (min-width: 600px)": { minHeight: 44 },
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
                it spans both panes and pushes them down when shown. Shared
                @shared-utils/ui visual (liveview's 3s-countdown + cache-clearing
                reload), bound to cowboy's connection store. */}
            <ConnectionBanner store={conn} />
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
                    // Frosted glass (translucent blur) like the compose/edit sheets.
                    // NOT `cover`: the session list is content-height — a full-screen
                    // sheet would leave empty frosted space below a short list. frosted
                    // works at either anchor (top on desktop, bottom on mobile).
                    frosted
                    surfaceColor={theme.palette.background.default}
                >
                    {/* DetentSheet's body has no side padding, so the list spans
                        the full width on its own — render it directly. (A former
                        `mx: -2` here "cancelled" a px:2 the sheet no longer has,
                        so it just bled the rows 16px PAST the viewport, clipping
                        the grip/kebab circles at the screen edge. The row's own
                        px gutter below insets the controls instead.) */}
                    {list}
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
                            // Wide hit area centred on the edge — easy to grab; the
                            // visible 1px accent line stays centred via `::after`.
                            right: -11,
                            width: 22,
                            height: "100%",
                            cursor: "col-resize",
                            zIndex: 2,
                            // Centered VSCode-style hairline: a SOLID 1px line
                            // (divider) that just recolours to the accent on hover /
                            // while dragging — never thickens. Identical to the
                            // composer-column divider below so the two read the same.
                            "&::after": {
                                content: '""',
                                position: "absolute",
                                top: 0,
                                left: "50%",
                                transform: "translateX(-50%)",
                                width: "1px",
                                height: "100%",
                                bgcolor: resizing ? "primary.main" : "divider",
                                transition: "background-color 120ms",
                            },
                            "&:hover::after": {
                                bgcolor: "primary.main",
                            },
                        }}
                    />
                </Stack>
            )}

            <Stack
                ref={columnRef}
                sx={{
                    flex: 1,
                    minWidth: 0,
                    // Anchor the floating frosted navbar + composer overlays (below)
                    // and clip them to the column. The overlay design (transcript is
                    // the full-height background, bars float over it as frosted glass)
                    // now applies in BOTH modes — desktop top-navbar included — so the
                    // clip is unconditional.
                    position: "relative",
                    overflow: "hidden",
                    // Lift the whole column off the on-screen keyboard + its
                    // iOS-native accessory bar: this padding (the keyboard's
                    // overlap, published by useKeyboardInset) reserves space at
                    // the bottom, so the flex:1 transcript shrinks and the bottom
                    // group (composer, or the navbar in bottom mode) rises clear
                    // of the keyboard. 0 when no keyboard.
                    pb: "var(--kb-inset, 0px)",
                }}
            >
                {/* Bottom-navbar mode leaves the TOP bare — the transcript runs
                    under the iOS status bar (time/signal/battery), which clashed
                    with the content. A frosted-glass strip over the safe-area-top
                    fixes it the iOS way: the status bar sits on blur, and content
                    scrolls UNDER it (the transcript's matching top inset clears it
                    at rest, so the glass only shows while scrolling). Height is
                    `env(safe-area-inset-top)` → 0 (invisible) off-device / on a
                    no-notch screen / when hosted, so it costs nothing there.
                    `pointer-events:none` so taps + scroll pass straight through.
                    Top mode doesn't need it — the AppBar already owns that edge. */}
                {/* TOP frosted glass. Bottom mode: a thin strip over the safe-area-top
                    so the iOS status bar sits on blur and content scrolls under it. Top
                    mode (desktop): a full navbar-height slab — the frosted glass BEHIND
                    the (now transparent) top AppBar, so the transcript diffuses under the
                    navbar exactly like the mobile bottom bar. Height tracks the measured
                    --navbar-h. pointer-events:none so taps reach the (lifted) navbar.
                    DROPPED in split mode: nothing scrolls under the AppBar (the transcript
                    is an in-flow column below it), so the bar just sits on the app surface. */}
                {!splitActive && (
                <Box
                    aria-hidden
                    sx={{
                        position: "absolute",
                        top: 0,
                        left: 0,
                        right: 0,
                        height: navbarAtBottom ? "env(safe-area-inset-top, 0px)" : "var(--navbar-h, 0px)",
                        // Behind the AppBar content (zIndex 2) but above the transcript (0).
                        // Bottom mode's strip sits alone at the top, so keep it high.
                        zIndex: navbarAtBottom ? ((t) => t.zIndex.appBar) : 1,
                        pointerEvents: "none",
                        // Hide under an open cover sheet — else this frosted strip
                        // bleeds through as a bright band under the status bar.
                        opacity: anySheetOpen ? 0 : 1,
                        transition: "opacity 200ms ease",
                        // Frosted / matte glass (磨砂): a milkier tint diffuses the content
                        // rather than showing it clearly; the heavy blur + `saturate` add
                        // the iOS-material vibrancy that reads as thick frosted glass.
                        // Bottom-mode (mobile) status-bar strip is OPAQUE-leaning (0.8, ≥
                        // the bottom slab's 0.76): `saturate` can only vivify the lavender
                        // backdrop, never a gray one, so at 0.62 a gray code block scrolling
                        // under it bled through as a muddy gray band under the status bar.
                        // A near-solid lavender tint masks that while staying frosted.
                        // Top mode (desktop, navbar at top) is a SOLID surface:
                        // an OPAQUE tint means the backdrop-blur has nothing to
                        // bleed, so the frosted edge can't read as a gray band
                        // under the bar (the reported "navbar 底部灰色阴影"). Bottom
                        // mode (mobile status-bar strip) stays frosted — content
                        // scrolls UNDER it, so it needs the translucent blur.
                        bgcolor: (t) =>
                            alpha(t.palette.background.default, navbarAtBottom ? 0.8 : 1),
                        ...(navbarAtBottom && {
                            backdropFilter: "blur(30px) saturate(200%)",
                            WebkitBackdropFilter: "blur(30px) saturate(200%)",
                        }),
                        // Desktop: a hairline delineates the navbar. In LIGHT mode the
                        // old `0 1px 24px` down-shadow smeared a gray cloud across the
                        // lavender (a black shadow on a light tint always reads gray) —
                        // dropped it; the hairline + the frosted blur are enough. Dark
                        // mode keeps a soft shadow (it reads as depth there, not gray).
                        ...(!navbarAtBottom && {
                            borderBottom: 1,
                            borderColor: "divider",
                            boxShadow: (t) =>
                                t.palette.mode === "dark" ? "0 1px 24px rgba(0,0,0,0.5)" : "none",
                        }),
                    }}
                />
                )}
                {/* ONE frosted-glass slab behind BOTH the composer and the navbar,
                    so they read as a single piece of glass — not two stacked panes
                    with a seam. Its height is exactly the measured panel height
                    (--composer-h + --navbar-h, the same vars the transcript reserves),
                    pinned to the bottom (above the keyboard inset). The composer +
                    navbar above it are made transparent (no own backdrop-filter), so
                    there's ONE blur context and no dividing line. zIndex 1 sits it
                    over the absolute transcript (0) and under the panel content (2);
                    pointer-events:none so taps fall through to that content. */}
                {/* BOTTOM frosted glass. Bottom mode: covers composer + navbar (one
                    continuous slab — they share the bottom edge). Top mode (desktop):
                    covers the composer only (the navbar is its own slab at the top), so
                    the composer/action bar floats as frosted glass with the transcript
                    diffusing under it. Height tracks the measured vars per mode.
                    DROPPED in split mode: the composer is a real left column, not a float,
                    so there's no glass to lay behind it. */}
                {!splitActive && (
                <Box
                    aria-hidden
                    sx={{
                            position: "absolute",
                            left: 0,
                            right: 0,
                            bottom: "var(--kb-inset, 0px)",
                            height: navbarAtBottom
                                ? "calc(var(--composer-h, 0px) + var(--navbar-h, 0px))"
                                : "var(--composer-h, 0px)",
                            zIndex: 1,
                            pointerEvents: "none",
                            // Hide under an open cover sheet — its own frosted surface
                            // replaces this chrome; leaving it on double-frosts.
                            opacity: anySheetOpen ? 0 : 1,
                            transition: "opacity 200ms ease, box-shadow 200ms ease",
                            // Milkier than a clear pane + heavy blur + saturate → thick
                            // iOS frosted material; content scrolling under it diffuses
                            // (not shows) through the blur. Up-shadow + top hairline give
                            // the "floating above the scroll" depth, now on the slab.
                            bgcolor: (t) => alpha(t.palette.background.default, t.palette.mode === "dark" ? 0.72 : 0.76),
                            backdropFilter: "blur(30px) saturate(200%)",
                            WebkitBackdropFilter: "blur(30px) saturate(200%)",
                            borderTop: 1,
                            borderColor: "divider",
                            // Up-shadow ("floating above the scroll" depth) ONLY when the
                            // transcript actually overflows under the glass. An empty/short
                            // conversation has nothing beneath it, so the shadow would read
                            // as a stray smudge under the header — gate it on real overflow.
                            boxShadow: transcriptScrollable
                                ? (t) =>
                                    `0 -1px 24px ${t.palette.mode === "dark" ? "rgba(0,0,0,0.5)" : "rgba(0,0,0,0.07)"}`
                                : "none",
                    }}
                />
                )}
                <AppBar
                    ref={appBarRef}
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
                        // Bottom mode: TRANSPARENT — the single frosted slab behind it
                        // (above) provides the glass, so the navbar + composer share ONE
                        // blur context with no seam. Just lift the content above the slab
                        // (zIndex 2 > slab's 1 > transcript's 0). Top mode keeps the solid
                        // themed surface (leading edge, nothing scrolls behind it).
                        // BOTH modes now: transparent + lifted above the transcript
                        // (zIndex 2 > the frosted slab's 1 > transcript's 0), so the
                        // slab behind provides the glass and content scrolls under the
                        // bar. (Was a solid `background.default` in top mode.)
                        bgcolor: "transparent",
                        position: "relative",
                        zIndex: 2,
                        color: "text.primary",
                        // Split mode: the frosted slab that drew the bar's bottom
                        // hairline is dropped, so the bar owns its own divider here
                        // (it's a solid top bar above the two columns, not a float).
                        ...(splitActive && { borderBottom: 1, borderColor: "divider" }),
                        // Bottom mode (mobile, navbar-pos=bottom): flex `order` puts
                        // the bar at the VERY bottom — below the transcript (0) AND
                        // the composer (1) — for a mobile-browser bottom-bar feel.
                        // Top mode: order 0, first child.
                        order: navbarAtBottom ? 2 : 0,
                        // Own the safe-area inset of whichever edge the bar hugs.
                        // Top: clear the status bar / notch. Bottom: the bar's buttons
                        // must clear the home-indicator zone — an earlier `home-inset −
                        // 20px` dropped the bar so low that the bottom-RIGHT buttons
                        // (settings gear, send) landed ON the iPad home indicator /
                        // rounded corner and were very hard to tap. Reserve the FULL
                        // safe-area inset (floored to 10px off-device) so the buttons
                        // sit just ABOVE the indicator — the standard, tappable iOS
                        // placement. Landscape rounded-corner side insets (pl/pr) keep
                        // the bar clear of the iPhone/iPad R角. env() insets are 0
                        // off-device + when hosted.
                        pt: navbarAtBottom ? 0 : "env(safe-area-inset-top, 0px)",
                        // Bottom gap = the home-indicator zone. The FULL inset (~34px)
                        // read as a big, lopsided gap vs the slim composer→navbar gap
                        // above; shave it so the bar sits closer to the edge while the
                        // 44px buttons still clear the indicator (an earlier `inset−20`
                        // ≈14px dropped them ONTO it — keep ≥ ~20px).
                        pb: navbarAtBottom ? "max(calc(env(safe-area-inset-bottom) - 18px), 12px)" : 0,
                        pl: navbarAtBottom ? "env(safe-area-inset-left, 0px)" : 0,
                        pr: navbarAtBottom ? "env(safe-area-inset-right, 0px)" : 0,
                    }}
                >
                    <Toolbar
                        variant="dense"
                        sx={{
                            // No border in either mode — the frosted slab behind the bar
                            // (top slab in desktop mode, bottom slab in mobile) owns the
                            // edge hairline + shadow, so a divider here would just be a
                            // double line / seam.
                            borderBottom: 0,
                            borderTop: 0,
                            borderColor: "divider",
                            // Hug the 44px icon row: the dense Toolbar's 48px min-height
                            // floated the icons with ~2px above, widening the gap to the
                            // composer's action row. Let the icons drive the height.
                            minHeight: 44,
                            "@media (min-width: 600px)": { minHeight: 44 },
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
                                // Unified 44px box + fixed 24px glyph (global
                                // MuiIconButton) keeps the hamburger aligned with
                                // the slash button below it at any font scale.
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
                            </Stack>
                        ) : (
                            // No session: the content pane already says "No
                            // session selected", so the bar shows nothing — no
                            // redundant brand/emoji.
                            <Box sx={{ flex: 1, minWidth: 0 }} />
                        )}
                        {/* Session-level controls (agent config / auto-scroll / Stop)
                            live here in the navbar, left of Settings — they act on the
                            session, not the message being typed, so they were pulled out
                            of the composer toolbar. */}
                        {active && (
                            <SessionControls
                                sessionId={active.id}
                                status={active.status}
                            />
                        )}
                        <IconButton
                            onClick={(): void => openSettings("settings")}
                            aria-label="settings"
                            title="Settings"
                            // Unified 44px box + 24px glyph (global MuiIconButton),
                            // so the gear stays aligned with the action row's
                            // send/stop button at any font scale.
                        >
                            <SettingsIcon />
                        </IconButton>
                    </Toolbar>
                </AppBar>

                {active ? (
                    splitActive ? (
                        // ===== Two-column split layout (desktop opt-in) =====
                        // A flex ROW below the in-flow AppBar (order 1): composer column
                        // (left, fixed width) | drag splitter | transcript (right, flex:1).
                        // The transcript is a normal in-flow column here — NOT the absolute
                        // overlay — so it reserves no --composer-h and the composer no longer
                        // floats over it. The AppStatusBar is a full-width footer (order 2).
                        <>
                            <Box
                                sx={{
                                    order: 1,
                                    flex: 1,
                                    minHeight: 0,
                                    display: "flex",
                                    flexDirection: "row",
                                    position: "relative",
                                }}
                            >
                                {/* Composer column (left) */}
                                <Box
                                    sx={{
                                        width: colWidth,
                                        flexShrink: 0,
                                        minWidth: 0,
                                        display: "flex",
                                        flexDirection: "column",
                                        minHeight: 0,
                                    }}
                                >
                                    {active.system ? (
                                        <Box sx={{ p: 1.5, textAlign: "center", fontSize: 13, opacity: 0.6 }}>
                                            View-only system session — managed by mnemosyne
                                        </Box>
                                    ) : (
                                        <Composer
                                            key={active.id}
                                            sessionId={active.id}
                                            status={active.status}
                                            onOpenInfo={(): void => openSettings("info")}
                                            variant="column"
                                        />
                                    )}
                                </Box>
                                {/* Vertical splitter — straddles the column edge so its
                                    6px hit area centres on the 1px divider line. */}
                                <Box
                                    role="separator"
                                    aria-orientation="vertical"
                                    aria-label="Resize composer column"
                                    onPointerDown={startColResize}
                                    sx={{
                                        flex: "0 0 auto",
                                        alignSelf: "stretch",
                                        // Identical VSCode-style hairline to the sidebar
                                        // divider above: a solid 1px line that recolours
                                        // to the accent on hover / drag, with the same
                                        // wide invisible hit area — so both read the same.
                                        width: "1px",
                                        bgcolor: colResizing ? "primary.main" : "divider",
                                        transition: "background-color 120ms",
                                        position: "relative",
                                        cursor: "col-resize",
                                        touchAction: "none",
                                        zIndex: 3,
                                        "&::after": {
                                            content: '""',
                                            position: "absolute",
                                            top: 0,
                                            bottom: 0,
                                            left: "-11px",
                                            right: "-11px",
                                        },
                                        "&:hover": { bgcolor: "primary.main" },
                                    }}
                                />
                                {/* Transcript column (right) */}
                                <Box
                                    sx={{
                                        flex: 1,
                                        minWidth: 0,
                                        position: "relative",
                                        display: "flex",
                                        flexDirection: "column",
                                    }}
                                >
                                    <Transcript
                                        sessionId={active.id}
                                        timeline={timelines.get(active.id) ?? []}
                                        status={active.status}
                                        provider={active.provider}
                                        cwd={active.cwd}
                                        loading={!hydrated.has(active.id)}
                                        connected={connected}
                                        // No floating chrome over this column: the AppBar is
                                        // in-flow above it and the composer is a sibling
                                        // column, so neither inset reserves anything.
                                        topInset="0px"
                                        bottomInset="0px"
                                    />
                                </Box>
                            </Box>
                            {/* Full-width status-bar footer spanning both columns. */}
                            <Box sx={{ order: 2, position: "relative", zIndex: 2 }}>
                                <AppStatusBar sessionId={active.id} status={active.status} />
                            </Box>
                        </>
                    ) : (
                    <>
                        {/* BOTH modes → the transcript is the FULL-height background
                            layer (absolute, zIndex 0) and the navbar + composer float
                            over it as frosted glass; content scrolls UNDER them. The
                            absolute layer is a flex column so the Transcript's own
                            flex:1 fills it. */}
                        <Box
                            sx={{ position: "absolute", inset: 0, zIndex: 0, display: "flex", flexDirection: "column" }}
                        >
                            <Transcript
                                sessionId={active.id}
                                timeline={timelines.get(active.id) ?? []}
                                status={active.status}
                                provider={active.provider}
                                cwd={active.cwd}
                                // Skeleton until this session's history snapshot
                                // lands (vs an empty session, which is hydrated).
                                loading={!hydrated.has(active.id)}
                                // While the WS is down the "working" spinner must not
                                // keep spinning on a stale status (daemon restart).
                                connected={connected}
                                // Gate the composer slab's up-shadow on real
                                // scroll-overflow (content under the glass).
                                onScrollableChange={setTranscriptScrollable}
                                // Reserve the top frosted bar at the transcript's top so
                                // content clears it at rest: the status-bar strip in
                                // mobile mode, the full navbar height in desktop mode.
                                topInset={navbarAtBottom ? "env(safe-area-inset-top, 0px)" : "var(--navbar-h, 0px)"}
                                // Reserve the floating bottom bar height (measured into CSS
                                // vars) so the newest message clears the glass at rest:
                                // composer+navbar in mobile mode, composer-only in desktop
                                // mode (the navbar is reserved at the top instead).
                                bottomInset={
                                    navbarAtBottom
                                        ? "calc(var(--composer-h, 0px) + var(--navbar-h, 0px))"
                                        : "var(--composer-h, 0px)"
                                }
                            />
                        </Box>
                        {/* A flex:1 spacer takes the place the (now absolute) transcript
                            vacated, so the floating bars sit at the column edges. Bottom
                            mode (order 0): pushes composer(1) + navbar(2) to the bottom.
                            Top mode (order 1): sits BETWEEN the navbar(0, top) and the
                            composer(2), pushing the composer to the bottom. */}
                        <Box aria-hidden sx={{ order: navbarAtBottom ? 0 : 1, flex: 1 }} />
                        {/* Composer order: mobile = 1 (above the bottom navbar); desktop =
                            2 (the very bottom, after the spacer). position:relative + zIndex
                            lifts the frosted composer above the absolute transcript layer. */}
                        <Box
                            ref={composerRef}
                            sx={{
                                order: navbarAtBottom ? 1 : 2,
                                minWidth: 0,
                                // Lift the composer content above the frosted slab behind
                                // it (zIndex 1) in BOTH modes — the composer is transparent,
                                // the slab is the glass, the transcript scrolls under both.
                                position: "relative",
                                zIndex: 2,
                            }}
                        >
                            {active.system ? (
                                <Box sx={{ p: 1.5, textAlign: "center", fontSize: 13, opacity: 0.6 }}>
                                    View-only system session — managed by mnemosyne
                                </Box>
                            ) : (
                                <Composer
                                    // Remount per session: each session owns its draft
                                    // (seeded from the per-session draft store) and a
                                    // fresh CodeMirror editor, so one session's
                                    // in-progress text never bleeds into another.
                                    key={active.id}
                                    sessionId={active.id}
                                    status={active.status}
                                    onOpenInfo={(): void => openSettings("info")}
                                />
                            )}
                            {/* Zed/VSCode-style status bar at the very bottom of
                                the window; inside this measured wrapper so the
                                transcript reserves it via --composer-h. */}
                            <AppStatusBar sessionId={active.id} status={active.status} />
                        </Box>
                    </>
                    )
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
                            {sessionsLoaded
                                ? (
                                    <>
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
                                    </>
                                )
                                : (
                                    // Until the WS snapshot lands, the list is unknown — show the
                                    // spinner, NOT the empty "No session selected." CTA, so a reload
                                    // never flashes a false "create your first session".
                                    <LoadingState />
                                )}
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
                        // Optimistic rename via the title-sync engine: the new
                        // title shows instantly and converges on every terminal.
                        renameSession(pendingRename.id, title);
                    }
                    setPendingRename(null);
                }}
            />
            <SessionInfoShell session={pendingInfo} onClose={(): void => setPendingInfo(null)} />
            <ConfirmSendModal />
            <ResourceLightbox />
            <JudgeInspectorHost forceSheet={navbarAtBottom} />
            <SettingsShell
                open={settingsOpen}
                onClose={(): void => setSettingsOpen(false)}
                initialTab={settingsTab}
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
// Sample values for the continuation-template live preview, so the preview works
// even with no real interruption on hand (the "示例数据" source).
const AUTO_RESUME_SAMPLE: Record<string, string> = {
    partial: "好的,我来重构鉴权模块。先抽出 token 校验:\n\n```ts\nexport function verifyToken(",
    prompt: "重构 auth 模块,把 token 校验抽成独立函数",
    cwd: "/home/draven/proj",
};
function interpolateTemplate(template: string, vars: Record<string, string>): string {
    // Mirror src/core.rs render_template: replace {{var}}; unknown vars stay
    // verbatim (so a typo is visible in the preview, not silently dropped).
    return template.replace(/\{\{(\w+)\}\}/g, (m, k: string) => vars[k] ?? m);
}

// Global auto-resume settings (tasks/active/session-auto-resume): the default
// toggle + a collapsed continuation-template editor with a live interpolated
// preview. Server-authoritative (reads `state.settings`, writes via setSetting).
function AutoResumeSettings(): React.JSX.Element {
    const settings = useStoreSelector((snapshot) => snapshot.settings);
    const defaultOn = settings[AUTO_RESUME_DEFAULT_KEY] === true;
    const saved =
        typeof settings[AUTO_RESUME_TEMPLATE_KEY] === "string"
            ? (settings[AUTO_RESUME_TEMPLATE_KEY] as string)
            : DEFAULT_CONTINUATION_TEMPLATE;
    const [editorOpen, setEditorOpen] = useState(false);
    const [draft, setDraft] = useState(saved);
    const dirty = draft !== saved;
    const preview = interpolateTemplate(draft, AUTO_RESUME_SAMPLE);
    // This disclosure sits near the bottom of the settings sheet, so expanding it
    // reveals the editor BELOW the fold. Pull the revealed content into view (next
    // frame, after it's laid out) so you don't have to hand-scroll. `block: "end"`
    // brings the bottom (the Save row) up; `nearest` would only reveal the top.
    const editorRef = useRef<HTMLDivElement>(null);
    useEffect(() => {
        if (!editorOpen) return undefined;
        const id = requestAnimationFrame(() => {
            editorRef.current?.scrollIntoView({ block: "end", behavior: "smooth" });
        });
        return () => cancelAnimationFrame(id);
    }, [editorOpen]);
    return (
        <Stack spacing={1}>
            <Typography variant="overline" color="text.secondary">
                Interrupted turns
            </Typography>
            <Stack
                direction="row"
                alignItems="center"
                justifyContent="space-between"
                spacing={2}
            >
                <Stack sx={{ minWidth: 0 }}>
                    <Typography variant="body2">Auto-resume interrupted turns</Typography>
                    <Typography variant="caption" color="text.secondary">
                        After a restart, an interrupted turn auto-continues with what it already
                        produced. May repeat side effects it already ran — enable only for tasks
                        that are safe to retry.
                    </Typography>
                </Stack>
                <Switch
                    checked={defaultOn}
                    onChange={(e): void => setSetting(AUTO_RESUME_DEFAULT_KEY, e.target.checked)}
                    inputProps={{ "aria-label": "Auto-resume interrupted turns" }}
                />
            </Stack>
            <Button
                size="small"
                color="inherit"
                onClick={(): void => setEditorOpen((v) => !v)}
                startIcon={editorOpen ? <ExpandLess /> : <ExpandMore />}
                sx={{ alignSelf: "flex-start", textTransform: "none", color: "text.secondary" }}
            >
                Customize continuation message
            </Button>
            {editorOpen && (
                <Stack ref={editorRef} spacing={1}>
                    <Typography variant="caption" color="text.secondary">
                        Variables: {"{{partial}}"} produced so far · {"{{prompt}}"} original
                        prompt · {"{{cwd}}"} working directory
                    </Typography>
                    <TextField
                        multiline
                        minRows={4}
                        maxRows={10}
                        fullWidth
                        value={draft}
                        onChange={(e): void => setDraft(e.target.value)}
                        slotProps={{ input: { sx: { fontSize: 13, fontFamily: "monospace" } } }}
                    />
                    <Stack direction="row" spacing={1}>
                        <Button
                            size="small"
                            variant="contained"
                            disabled={!dirty}
                            onClick={(): void => setSetting(AUTO_RESUME_TEMPLATE_KEY, draft)}
                        >
                            Save
                        </Button>
                        <Button
                            size="small"
                            color="inherit"
                            onClick={(): void => {
                                setDraft(DEFAULT_CONTINUATION_TEMPLATE);
                                setSetting(AUTO_RESUME_TEMPLATE_KEY, DEFAULT_CONTINUATION_TEMPLATE);
                            }}
                        >
                            Reset to default
                        </Button>
                    </Stack>
                    <Typography variant="caption" color="text.secondary">
                        Preview (sample data)
                    </Typography>
                    <Box
                        sx={{
                            p: 1,
                            borderRadius: 1,
                            border: 1,
                            borderColor: "divider",
                            bgcolor: "action.hover",
                            fontSize: 13,
                            whiteSpace: "pre-wrap",
                            maxHeight: 200,
                            overflowY: "auto",
                        }}
                    >
                        {preview}
                    </Box>
                </Stack>
            )}
        </Stack>
    );
}

function SettingsShell({
    open,
    onClose,
    initialTab,
    themeMode,
    onSetThemeMode,
}: {
    open: boolean;
    onClose: () => void;
    initialTab: "settings" | "info";
    themeMode: ThemeMode;
    onSetThemeMode: (m: ThemeMode) => void;
}): React.JSX.Element {
    // Merged sheet: a Settings / Info segmented switch in the header. Each open
    // lands on the tab whose button was tapped (gear → settings, ℹ️ → info).
    const [tab, setTab] = useState<"settings" | "info">(initialTab);
    useEffect(() => {
        if (open) setTab(initialTab);
    }, [open, initialTab]);
    const vim = useVimSetting();
    const notify = useNotifySetting();
    const vibrate = useVibrateSetting();
    const desktopLayout = useDesktopLayout();
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
    const navbarAtBottom = useNavbarAtBottom();
    // Vim is desktop-only (ComposerEditor won't load it on touch), so the
    // toggle only appears where a physical keyboard exists.
    const desktop = useMediaQuery("(pointer: fine) and (hover: hover)");
    return (
        <Sheet
            open={open}
            onClose={onClose}
            forceSheet={navbarAtBottom}
            cover
        >
            {/* Tab switcher + close, rendered in the BODY rather than the shared
                sheet title row. That row pairs the title with a fixed 40px close
                button, so its height + padding forced an over-tall header with too
                much space above/below the pill; here cowboy owns the spacing. The
                flex:1 spacers either side keep the pill dead-centred with the close
                at the trailing edge; mt/mb tune the top/bottom whitespace directly. */}
            <Box sx={{ display: "flex", alignItems: "center", mt: 0.25, mb: 1.5 }}>
                <Box sx={{ flex: 1 }} />
                <SegmentedPill
                    value={tab}
                    onChange={setTab}
                    options={[{ value: "settings", label: "Settings" }, { value: "info", label: "Info" }]}
                />
                <Box sx={{ flex: 1, display: "flex", justifyContent: "flex-end" }}>
                    <IconButton aria-label="close settings" onClick={onClose} sx={{ width: 36, height: 36 }}>
                        <CloseIcon fontSize="small" />
                    </IconButton>
                </Box>
            </Box>
            {tab === "info" ? <InfoContent /> : (
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
                        <Typography variant="body2">Sound alert</Typography>
                        <Typography variant="caption" color="text.secondary">
                            Chime when an agent finishes or needs you
                        </Typography>
                    </Stack>
                    <Switch
                        checked={notify}
                        onChange={(e): void => setNotifySetting(e.target.checked)}
                        inputProps={{ "aria-label": "Sound alert" }}
                    />
                </Stack>
                <Stack
                    direction="row"
                    alignItems="center"
                    justifyContent="space-between"
                    spacing={2}
                >
                    <Stack>
                        <Typography variant="body2">Vibration alert</Typography>
                        <Typography variant="caption" color="text.secondary">
                            Vibrate when an agent finishes or needs you (native on
                            iPhone, no-op on devices without a vibration motor)
                        </Typography>
                    </Stack>
                    <Switch
                        checked={vibrate}
                        onChange={(e): void => setVibrateSetting(e.target.checked)}
                        inputProps={{ "aria-label": "Vibration alert" }}
                    />
                </Stack>
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
                        <Stack
                            direction="row"
                            alignItems="center"
                            justifyContent="space-between"
                            spacing={2}
                        >
                            <Stack>
                                <Typography variant="body2">
                                    Two-column layout
                                </Typography>
                                <Typography
                                    variant="caption"
                                    color="text.secondary"
                                >
                                    Composer beside the chat (Zed-style); wide
                                    screens only
                                </Typography>
                            </Stack>
                            <Switch
                                checked={desktopLayout === "split"}
                                onChange={(e): void =>
                                    setDesktopLayout(
                                        e.target.checked ? "split" : "overlay",
                                    )}
                                inputProps={{
                                    "aria-label": "Two-column layout",
                                }}
                            />
                        </Stack>
                    </>
                )}
                <Divider />
                <AutoResumeSettings />
                {/* Daemon system info (Storage metrics + About) lives in the Info
                    tab — Settings holds user preferences only. */}
            </Stack>
            )}
        </Sheet>
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
    // Hooks before the early return (rules of hooks).
    const navbarAtBottom = useNavbarAtBottom();
    useConfirmEnter(session !== null, onConfirm);
    if (!session) return null;
    const surface = originLabel(session.origin);
    return (
        <Sheet
            forceSheet={navbarAtBottom}
            open
            onClose={onClose}
            title={`Delete this ${surface} session?`}
            actions={
                <>
                    <Button onClick={onClose} color="inherit">
                        Cancel
                        <Kbd keys="Esc" />
                    </Button>
                    <Button onClick={onConfirm} color="error" variant="contained" autoFocus>
                        Delete
                        <Kbd keys={ENTER_LABEL} />
                    </Button>
                </>
            }
        >
            <Typography variant="body2" color="text.secondary">
                Any in-flight turn is cancelled. The agent transcript on this session will be lost.
            </Typography>
        </Sheet>
    );
}

// Prefills the textfield with the current title; Save is disabled while empty
// or unchanged (server-side also rejects empty). The rename + compose sheets both
// raise the keyboard in-gesture via `claimKeyboard` (see ./keyboardClaim).

// The connecting/loading placeholder — SKELETONS that mirror the real layout
// (a settling list of items in the sidebar, a settling transcript in the main
// pane) instead of a bare spinner: it reads as "content arriving", not "stuck".
// If the WS snapshot still hasn't landed after a stall (a wedged socket / SW), it
// reveals a manual reload so a session never sits dead forever. NOT auto-reload:
// an auto-loop could spin if the reload also stalls.
function LoadingState({ compact = false }: { compact?: boolean }): React.JSX.Element {
    const [stalled, setStalled] = useState(false);
    useEffect(() => {
        const t = globalThis.setTimeout(() => setStalled(true), 8000);
        return () => globalThis.clearTimeout(t);
    }, []);
    const reloadButton = stalled
        ? (
            <Button
                size="small"
                variant="outlined"
                onClick={(): void => globalThis.location.reload()}
                sx={{ alignSelf: compact ? "stretch" : "center", mt: 0.5, textTransform: "none" }}
            >
                Taking a while — reload
            </Button>
        )
        : null;
    // Sidebar: a short stack of rounded rows standing in for session items, fading
    // down so the list reads as "settling" rather than a wall of identical bars.
    if (compact) {
        return (
            <Stack spacing={0.75} sx={{ px: 1, py: 1 }} aria-label="Loading sessions">
                {[0, 1, 2, 3, 4].map((i) => (
                    <Skeleton
                        key={i}
                        variant="rounded"
                        height={36}
                        animation="wave"
                        sx={{ borderRadius: 1.5, opacity: 1 - i * 0.16 }}
                    />
                ))}
                {reloadButton}
            </Stack>
        );
    }
    // Main pane: a few transcript-shaped blocks (a short label + a message bubble),
    // alternating width + fading down, so it previews the conversation layout.
    return (
        <Stack
            spacing={2}
            // EXPLICIT width (not width:100%): this renders inside a
            // `width: max-content` centred container, where width:100% +
            // intrinsic-less skeletons collapse to a thin sliver (the reported
            // "thin line" bug). A fixed min(720, 90vw) gives the parent a definite
            // width so the bubbles fill it.
            sx={{ width: "min(720px, 90vw)", px: 2, py: 3 }}
            aria-label="Loading conversation"
        >
            {[0, 1, 2].map((row) => (
                <Stack key={row} spacing={0.75} sx={{ width: "100%", opacity: 1 - row * 0.2 }}>
                    <Skeleton
                        variant="text"
                        width={row % 2 === 0 ? 180 : 130}
                        animation="wave"
                        sx={{ fontSize: "0.85rem" }}
                    />
                    <Skeleton
                        variant="rounded"
                        width="100%"
                        height={row % 2 === 0 ? 72 : 52}
                        animation="wave"
                        sx={{ borderRadius: 2 }}
                    />
                </Stack>
            ))}
            {reloadButton}
        </Stack>
    );
}

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
        // The keyboard was already claimed in-gesture (claimKeyboard, on the rename
        // tap) so it's rising regardless; here we just TRANSFER focus to the real
        // field + select-all (you're almost always retyping). A short delay lets the
        // field mount and its value commit before select(); the transfer keeps the
        // keyboard up (the DetentSheet is bottom-anchored + not a Modal, so it tracks
        // the keyboard and never steals focus). Snappy so it beats any menu
        // focus-restore that would otherwise blur the claim before the transfer.
        const t = globalThis.setTimeout(() => {
            inputRef.current?.focus();
            inputRef.current?.select();
        }, 120);
        return () => globalThis.clearTimeout(t);
    }, [session?.id, session?.title]);
    if (!session) return null;
    const trimmed = value.trim();
    const canSave = trimmed.length > 0 && trimmed !== session.title;
    const submit = (): void => {
        if (canSave) onConfirm(trimmed);
    };
    return (
        <Sheet
            forceSheet={navbarAtBottom}
            open
            onClose={onClose}
            title="Rename session"
            actions={
                <>
                    <Button onClick={onClose} color="inherit">
                        Cancel
                        <Kbd keys="Esc" />
                    </Button>
                    <Button onClick={submit} variant="contained" disabled={!canSave}>
                        Save
                        <Kbd keys={ENTER_LABEL} />
                    </Button>
                </>
            }
        >
            <TextField
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
        </Sheet>
    );
}

// --- Info: per-session dialog + the Settings storage panel ------------------
// Both read the daemon's HTTP info endpoints (GET, no WS) and surface the
// capacity numbers the audit flagged — events are the unbounded grower.

// A label/value row used by both info surfaces.
function InfoRow({ k, v }: { k: string; v: string }): React.JSX.Element {
    return (
        <Stack direction="row" spacing={2} sx={{ justifyContent: "space-between", alignItems: "baseline" }}>
            <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0 }}>{k}</Typography>
            <Typography variant="body2" sx={{ wordBreak: "break-all", textAlign: "right" }}>{v}</Typography>
        </Stack>
    );
}

interface SessionInfoData {
    id: string;
    provider: string;
    cwd: string;
    title: string;
    status: string;
    event_count: number;
    queue_count: number;
    drafts_count: number;
}

function SessionInfoShell(
    { session, onClose }: { session: SessionMeta | null; onClose: () => void },
): React.JSX.Element | null {
    const navbarAtBottom = useNavbarAtBottom();
    const [info, setInfo] = useState<SessionInfoData | null>(null);
    const [error, setError] = useState(false);
    useEffect(() => {
        if (!session) return undefined;
        setInfo(null);
        setError(false);
        const ctrl = new AbortController();
        void fetch(`/api/sessions/${encodeURIComponent(session.id)}/info`, { signal: ctrl.signal })
            .then((r) => (r.ok ? (r.json() as Promise<SessionInfoData>) : Promise.reject(new Error("not found"))))
            .then(setInfo)
            .catch(() => {
                if (!ctrl.signal.aborted) setError(true);
            });
        return () => {
            ctrl.abort();
        };
    }, [session?.id]);
    if (!session) return null;
    let body: React.JSX.Element;
    if (error) {
        body = <Typography color="error" variant="body2" sx={{ p: 1 }}>Couldn't load session info.</Typography>;
    } else if (!info) {
        body = <Typography variant="body2" sx={{ p: 1, color: "text.secondary" }}>Loading…</Typography>;
    } else {
        body = (
            <Stack spacing={1} sx={{ mt: 1 }}>
                <InfoRow k="Title" v={info.title} />
                <InfoRow k="Status" v={info.status} />
                <InfoRow k="Provider" v={info.provider} />
                <InfoRow k="Directory" v={info.cwd} />
                <InfoRow k="Events" v={info.event_count.toLocaleString()} />
                <InfoRow k="Queued" v={String(info.queue_count)} />
                <InfoRow k="Drafts" v={String(info.drafts_count)} />
                <InfoRow k="ID" v={info.id} />
            </Stack>
        );
    }
    return (
        <Sheet
            forceSheet={navbarAtBottom}
            open
            onClose={onClose}
            title="Session info"
            actions={<Button onClick={onClose} color="inherit">Close<Kbd keys="Esc" /></Button>}
        >
            {body}
        </Sheet>
    );
}
