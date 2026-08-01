import {
    forwardRef,
    lazy,
    memo,
    startTransition,
    Suspense,
    useCallback,
    useEffect,
    useLayoutEffect,
    useRef,
    useState,
} from "react";
import type { ComponentPropsWithoutRef } from "react";
import {
    Alert,
    alpha,
    AppBar,
    Box,
    Button,
    ButtonBase,
    Chip,
    CircularProgress,
    Dialog,
    DialogContent,
    DialogTitle,
    Divider,
    IconButton,
    List,
    ListItemButton,
    ListItemIcon,
    ListItemText,
    ListSubheader,
    Menu,
    MenuItem,
    Paper,
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
    Refresh as RefreshIcon,
    Schedule,
    Settings as SettingsIcon,
    SystemUpdateAlt,
} from "@mui/icons-material";
import { SessionControls } from "./Composer";
import { MobileComposer } from "./mobile/MobileComposer";
import { claimKeyboard } from "./keyboardClaim";
import { machineProviderAvailable } from "./machineProvider";
import { machineVersionPresentation, type MachineComponentUpdate } from "./machineVersions";
import { Transcript } from "./Transcript";
import { desktopScrollbarSx } from "./desktop/desktopScrollbar";
import {
    PROVIDERS,
    type Envelope,
    type SessionMeta,
    type Status,
} from "./protocol";
import {
    AUTO_RESUME_DEFAULT_KEY,
    AUTO_RESUME_TEMPLATE_KEY,
    DEFAULT_CONTINUATION_TEMPLATE,
    holdStorePresentation,
    markSessionHydrated,
    notify,
    openSession,
    renameSession,
    releaseInactiveHistory,
    reorderSessions,
    send,
    setSessionAutoResume,
    setSetting,
    useStoreSelector,
} from "./store";
import { useSortable } from "./useSortable";
import { useReliableTouchTap } from "./useReliableTouchTap";
import { bindMobileSpatialDrawer } from "./mobileSpatialDrawer";
import { setNotifySetting, setVibrateSetting, useNotifySetting, useVibrateSetting } from "./turnNotify";
import {
    clampComposerColWidth,
    composerColWidthStore,
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
import {
    type MachinePresence,
    machinePresencePresentation,
} from "./machinePresence";
import {
    ExploreTranscript,
    MobilePageDock,
} from "./explore/ExploreSurface";
import {
    setTranscriptProjection,
    captureTranscriptViewportAnchor,
    resolveProjectionAnchor,
    queueExploreFollowUp,
    useExploreSessionState,
} from "./explore/exploreStore";
import { retainTranscriptViewportSessions } from "./transcriptViewportStore";
import {
    ConnectionBanner,
    DetentSheet,
    MobileSheetActionGroup,
    MobileSheetDismiss,
    NativeReleaseUpdatePrompt,
    ThemeModeControl,
    useAnyDetentSheetOpen,
} from "./_shell";
import { Sheet } from "./Sheet";
import { Kbd, useConfirmEnter } from "./Kbd";
import { isImeKeyEvent } from "./imeKey";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { InfoContent } from "./InfoSheet";
import { UsageLogs } from "./UsageLogs";
import { SegmentedPill } from "./SegmentedPill";
import { fireLabel, fireRel } from "./scheduleTime";
import { ResourceLightbox } from "./ResourceLightbox";
import { JudgeInspectorHost } from "./JudgeInspector";
import { desktopFocusBoundary, desktopFocusFill, type Mode as ThemeMode } from "./theme";
import { persisted } from "./_store/mod.ts";
import { workspaceCommandKey } from "./desktop/commands/workspaceCommandKey";
import { sequentialShortcutAvailability } from "./desktop/commands/shortcutAvailability";
import {
    desktopEmbeddedControlSx,
    desktopListItemSx,
} from "./desktop/DesktopEmbeddedControl";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import {
    controlPlaneConnection,
    getActiveSessionId,
    setActiveSessionId,
    useActiveSessionId,
} from "./controlPlane";
import { defaultNewSessionWorkspace } from "./newSessionWorkspace";
import { DesktopShortcutBar } from "./desktop/DesktopShortcutBar";
import { DesktopModal as DesktopModalShell } from "./desktop/DesktopModal";

const DesktopCommandHost = lazy(async () => {
    const module = await import("./desktop/commands/DesktopCommandHost");
    return { default: module.DesktopCommandHost };
});
const DesktopStatusLine = lazy(async () => {
    const module = await import("./desktop/DesktopStatusLine");
    return { default: module.DesktopStatusLine };
});
const DesktopTopBarControls = lazy(async () => {
    const module = await import("./desktop/DesktopTopBarControls");
    return { default: module.DesktopTopBarControls };
});
const DesktopComposer = lazy(async () => {
    const module = await import("./desktop/DesktopComposer");
    return { default: module.DesktopComposer };
});
const DesktopSessionShortcut = lazy(async () => {
    const module = await import("./desktop/DesktopSessionShortcut");
    return { default: module.DesktopSessionShortcut };
});
const DesktopWorkspace = lazy(async () => {
    const module = await import("./desktop/DesktopWorkspace");
    return { default: module.DesktopWorkspace };
});
const DesktopRegionShortcut = lazy(async () => {
    const module = await import("./desktop/DesktopRegionShortcut");
    return { default: module.DesktopRegionShortcut };
});
const DesktopContextShortcut = lazy(async () => {
    const module = await import("./desktop/commands/DesktopContextShortcut");
    return { default: module.DesktopContextShortcut };
});
// Desktop sidebar width: a user-draggable pixel width (VSCode-style divider),
// persisted in localStorage. The bounds keep both panes usable — 240px floor
// (a list row stays readable), 480px ceiling (wider and the list looks empty
// next to the transcript). 300px default sits inside the old fluid
// `clamp(240px, 22vw, 360px)` it replaces. Resize is desktop-only: below the
// `lg` breakpoint the sidebar is a full-width top Drawer (touch layout), which
// has no divider — so neither bound nor handle applies there.
// App-wide bottom status bar (Zed / VSCode style): a thin strip at the very
// bottom of the window. DESKTOP ONLY + only when vim is on (its sole status today
const SIDEBAR_MIN = 240;
const SIDEBAR_MAX = 480;
const SIDEBAR_DEFAULT = 288;

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
                disableShrink
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

// Auto-resume indicator (tasks/archive/2026/07/session-auto-resume). Shown whenever the
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
                    disableShrink
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

function SessionProjectionBadge({
    sessionId,
}: {
    sessionId: string;
}): React.JSX.Element | null {
    const { projection } = useExploreSessionState(sessionId);
    if (projection !== "explore") return null;
    return (
        <Chip
            size="small"
            label="PAGE"
            aria-label="Question page view"
            color="primary"
            sx={{
                height: "1.65em",
                minWidth: "4.2em",
                flexShrink: 0,
                bgcolor: (theme) => alpha(theme.palette.primary.main, 0.1),
                color: "primary.main",
                fontSize: "0.72em",
                fontWeight: 700,
                letterSpacing: "0.06em",
                "& .MuiChip-label": {
                    px: "0.65em",
                    overflow: "visible",
                },
            }}
        />
    );
}

const ReliableListItemButton = forwardRef<
    HTMLDivElement,
    Omit<ComponentPropsWithoutRef<typeof ListItemButton>, "onClick"> & { onActivate: () => void }
>(function ReliableListItemButton({ onActivate, sx, ...props }, ref) {
    const tap = useReliableTouchTap<HTMLDivElement>(onActivate);
    return (
        <ListItemButton
            {...props}
            {...tap}
            ref={ref}
            sx={[
                { touchAction: "manipulation" },
                ...(Array.isArray(sx) ? sx : [sx]),
            ]}
        />
    );
});

function SessionList({
    sessions,
    activeId,
    onPick,
    onNew,
    onClose,
    onRequestDelete,
    onRequestInfo,
    onRequestRename,
    autoResumeDefault,
    loaded,
    desktop,
    mobileDrawer = false,
    mobileDrawerOpen = false,
    phone = false,
}: {
    sessions: SessionMeta[];
    activeId: string | null;
    onPick: (id: string) => void;
    onNew: () => void;
    onClose?: (() => void) | undefined;
    onRequestDelete: (s: SessionMeta) => void;
    onRequestInfo: (s: SessionMeta) => void;
    onRequestRename: (s: SessionMeta) => void;
    autoResumeDefault: boolean;
    // True once the first session list has arrived over the WS. Until then the
    // list is genuinely UNKNOWN (not "empty") — distinguishing the two avoids the
    // false "No sessions yet." flash on a reload before the snapshot lands.
    loaded: boolean;
    desktop: boolean;
    mobileDrawer?: boolean;
    mobileDrawerOpen?: boolean;
    phone?: boolean;
}): React.JSX.Element {
    // Desktop-only modal list state. Normal mode navigates with j/k; Pin turns
    // the same keys into spatial reorder commands until P/Esc (or opening a
    // session) releases it. This is intentionally local UI state, not synced
    // session metadata.
    const [pinned, setPinned] = useState(false);
    // Per-row kebab Menu anchor + target. Standard Material list-row
    // pattern: trailing IconButton with MoreVert opens a Menu containing
    // Rename + Delete — two-step gesture (open menu, pick item, confirm
    // dialog) replaces the previous swipe-to-delete, which the user
    // (correctly) flagged as mis-tap-prone.
    const [menuAnchor, setMenuAnchor] = useState<{
        row: SessionMeta;
        el: HTMLElement;
    } | null>(null);
    const menuProjection = useExploreSessionState(
        menuAnchor?.row.id ?? "__session-menu-none__",
    ).projection;
    const setMenuProjection = (projection: "history" | "explore"): void => {
        if (!menuAnchor) return;
        const sessionId = menuAnchor.row.id;
        const anchor = captureTranscriptViewportAnchor(sessionId);
        setTranscriptProjection(sessionId, projection, anchor);
        setMenuAnchor(null);
    };
    // Drag-to-reorder via the leading grip handle (server-authoritative, synced).
    const byId = new Map(sessions.map((s) => [s.id, s]));
    const displayedSessions = mobileDrawer ? [...sessions].reverse() : sessions;
    const listRef = useRef<HTMLUListElement>(null);
    const sortable = useSortable({
        ids: displayedSessions.map((s) => s.id),
        onReorder: (order): void =>
            reorderSessions(mobileDrawer ? [...order].reverse() : order),
        scrollContainer: () => listRef.current,
    });
    const positionedMobileOpenRef = useRef(false);
    useLayoutEffect(() => {
        if (!mobileDrawer || !mobileDrawerOpen) {
            positionedMobileOpenRef.current = false;
            return undefined;
        }
        if (positionedMobileOpenRef.current || !loaded) return undefined;
        positionedMobileOpenRef.current = true;
        let frame = requestAnimationFrame(() => {
            frame = requestAnimationFrame(() => {
                const list = listRef.current;
                if (list) list.scrollTop = list.scrollHeight - list.clientHeight;
            });
        });
        return () => cancelAnimationFrame(frame);
    }, [loaded, mobileDrawer, mobileDrawerOpen]);
    useEffect(() => {
        const list = listRef.current;
        const region = list?.closest<HTMLElement>("[data-desktop-region='sessions.list']");
        if (!list || !region || !desktop) return undefined;
        region.dataset.desktopPinned = pinned ? "true" : "false";
        const onTogglePin = (): void => setPinned((current) => !current);
        const onReleasePin = (): void => setPinned(false);
        list.addEventListener("cowboy:desktop-toggle-pin", onTogglePin);
        list.addEventListener("cowboy:desktop-release-pin", onReleasePin);
        return () => {
            delete region.dataset.desktopPinned;
            list.removeEventListener("cowboy:desktop-toggle-pin", onTogglePin);
            list.removeEventListener("cowboy:desktop-release-pin", onReleasePin);
        };
    }, [desktop, pinned]);
    useEffect(() => {
        const list = listRef.current;
        if (!list) return undefined;
        const onKeyboardReorder = (event: Event): void => {
            const detail = (event as CustomEvent<{ delta?: number }>).detail;
            const item = event.target instanceof Element
                ? event.target.closest<HTMLElement>("[data-desktop-item]")
                : null;
            const id = item?.dataset.desktopItem;
            const delta = detail?.delta;
            if (!id || (delta !== -1 && delta !== 1)) return;
            const current = sortable.order.indexOf(id);
            const next = Math.max(0, Math.min(sortable.order.length - 1, current + delta));
            if (current < 0 || current === next) return;
            const order = [...sortable.order];
            order.splice(current, 1);
            order.splice(next, 0, id);
            reorderSessions(order);
            requestAnimationFrame(() =>
                list.querySelector<HTMLElement>(
                    `[data-desktop-item="${CSS.escape(id)}"]`,
                )?.focus({ preventScroll: true })
            );
        };
        list.addEventListener("cowboy:desktop-reorder", onKeyboardReorder);
        return () => list.removeEventListener("cowboy:desktop-reorder", onKeyboardReorder);
    }, [sortable.order]);
    useEffect(() => {
        const list = listRef.current;
        if (!list || !desktop) return undefined;
        const onKeyboardSettings = (event: Event): void => {
            const item = event.target instanceof Element
                ? event.target.closest<HTMLElement>("[data-desktop-item]")
                : null;
            const session = item?.dataset.desktopItem
                ? byId.get(item.dataset.desktopItem)
                : undefined;
            if (session) setMenuAnchor({ row: session, el: item ?? list });
        };
        list.addEventListener("cowboy:desktop-session-settings", onKeyboardSettings);
        return () =>
            list.removeEventListener("cowboy:desktop-session-settings", onKeyboardSettings);
    }, [byId, desktop]);
    useEffect(() => {
        if (!desktop || !menuAnchor) return undefined;
        const frame = requestAnimationFrame(() => {
            document.querySelector<HTMLButtonElement>(
                "[data-desktop-session-actions] button:not(:disabled)",
            )?.focus({ preventScroll: true });
        });
        return () => cancelAnimationFrame(frame);
    }, [desktop, menuAnchor?.row.id]);
    const onDesktopListKeyDown = (event: React.KeyboardEvent<HTMLUListElement>): void => {
        if (
            !desktop || event.metaKey || event.ctrlKey || event.altKey ||
            event.shiftKey || event.repeat
        ) return;
        const key = workspaceCommandKey(event.nativeEvent);
        const rows = [...(listRef.current?.querySelectorAll<HTMLElement>(
            "[data-desktop-item]",
        ) ?? [])].filter((row) => row.offsetParent !== null);
        const focused = event.target instanceof Element
            ? event.target.closest<HTMLElement>("[data-desktop-item]")
            : null;
        const current = Math.max(0, focused ? rows.indexOf(focused) : -1);
        const row = rows[current];
        if (!row) return;
        if (key === "j" || key === "k") {
            event.preventDefault();
            event.stopPropagation();
            if (pinned) {
                row.dispatchEvent(new CustomEvent("cowboy:desktop-reorder", {
                    bubbles: true,
                    detail: { delta: key === "j" ? 1 : -1 },
                }));
                return;
            }
            const next = Math.min(
                rows.length - 1,
                Math.max(0, current + (key === "j" ? 1 : -1)),
            );
            rows[next]?.focus({ preventScroll: true });
            rows[next]?.scrollIntoView({ block: "nearest" });
            return;
        }
        if (key === "p") {
            event.preventDefault();
            event.stopPropagation();
            setPinned((value) => !value);
            row.focus({ preventScroll: true });
            return;
        }
        if (key === "Escape" && pinned) {
            event.preventDefault();
            event.stopPropagation();
            setPinned(false);
            return;
        }
        const session = row.dataset.desktopItem
            ? byId.get(row.dataset.desktopItem)
            : undefined;
        if (key === "h" && session) {
            event.preventDefault();
            event.stopPropagation();
            setMenuAnchor({ row: session, el: row });
            return;
        }
        if ((key === "l" || key === "Enter") && session) {
            // Let a focused grip or kebab retain native Enter activation.
            if (key === "Enter" && event.target !== row) return;
            event.preventDefault();
            event.stopPropagation();
            setPinned(false);
            onPick(session.id);
            requestAnimationFrame(() => {
                const prompt = document.querySelector<HTMLElement>(
                    "[data-desktop-region='prompt.composer']",
                );
                const target = prompt?.querySelector<HTMLElement>(
                    "[data-vim-command-sink], .cm-content[contenteditable='true']",
                );
                target?.focus({ preventScroll: true });
            });
        }
    };
    return (
        <Stack
            sx={{
                // DetentSheet keeps trailing clearance for generic overlay
                // footers. Sessions owns an inner scrolling list, so consume
                // that clearance here: the list continues behind the floating
                // Close island instead of ending in a footer-shaped blank band.
                height: desktop || mobileDrawer
                    ? "100%"
                    : "calc(100% + 76px + env(safe-area-inset-bottom, 0px))",
                mb: desktop || mobileDrawer
                    ? 0
                    : "calc(-76px - env(safe-area-inset-bottom, 0px))",
                position: mobileDrawer ? "relative" : undefined,
                minHeight: 0,
            }}
        >
            {!mobileDrawer && <Box sx={{ p: 1 }}>
                <Button
                    data-desktop-new-session={desktop ? "true" : undefined}
                    fullWidth
                    variant="outlined"
                    startIcon={<Add />}
                    onClick={onNew}
                    sx={desktop ? {
                        ...desktopEmbeddedControlSx(),
                        minHeight: 48,
                        textTransform: "uppercase",
                        letterSpacing: "0.08em",
                    } : undefined}
                >
                    New session
                    {desktop && <Kbd keys={`${MOD_LABEL}N`} variant="global" />}
                </Button>
                {desktop && pinned && (
                    <Box
                        role="status"
                        sx={{
                            mt: 0.75,
                            minHeight: 32,
                            px: 1,
                            display: "flex",
                            alignItems: "center",
                            gap: 0.75,
                            border: 1,
                            borderColor: "primary.main",
                            borderRadius: 1.25,
                            color: "primary.main",
                            bgcolor: (theme) => alpha(theme.palette.primary.main, 0.055),
                        }}
                    >
                        <Typography
                            variant="caption"
                            sx={{ fontWeight: 750, letterSpacing: "0.045em" }}
                        >
                            REORDER
                        </Typography>
                        <Box sx={{ flex: 1 }} />
                        <Kbd keys="J/K" />
                        <Typography variant="caption" color="text.secondary">Move</Typography>
                        <Kbd keys="Esc" />
                        <Typography variant="caption" color="text.secondary">Done</Typography>
                    </Box>
                )}
            </Box>}
            <List
                dense
                ref={listRef}
                onKeyDownCapture={onDesktopListKeyDown}
                sx={{
                    flex: 1,
                    overflowY: "auto",
                    ...(desktop ? desktopScrollbarSx : {}),
                    // The dismiss island overlays rows during ordinary scrolling,
                    // but at the true end the final session must be able to rest
                    // fully above it. Keep this clearance inside the scrolling
                    // list (like Settings), not as a fixed footer-sized gap.
                    pb: desktop
                        ? undefined
                        : mobileDrawer
                        ? "calc(84px + env(safe-area-inset-bottom, 0px))"
                        : "calc(76px + env(safe-area-inset-bottom, 0px))",
                    // Fine-pointer desktops do not need phone-sized 44px controls
                    // in every row. Keep the generous targets for touch/tablet,
                    // while fitting more sessions without making the rail noisy.
                    "@media (pointer: fine) and (hover: hover)": {
                        py: 0.5,
                        "& .cowboy-session-grip, & .cowboy-session-actions": {
                            width: 32,
                            height: 32,
                        },
                        "& .cowboy-session-grip .MuiSvgIcon-root, & .cowboy-session-actions .MuiSvgIcon-root": {
                            fontSize: "1.125rem",
                        },
                    },
                }}
            >
                {sortable.order.map((id, index) => {
                    const s = byId.get(id);
                    if (!s) return null;
                    return (
                    <ReliableListItemButton
                        key={s.id}
                        data-desktop-item={s.id}
                        data-desktop-session-row={desktop ? "true" : undefined}
                        data-desktop-current={desktop && s.id === activeId ? "true" : undefined}
                        data-desktop-pin-active={desktop && pinned ? "true" : undefined}
                        ref={sortable.registerItem(s.id)}
                        style={sortable.itemStyle(s.id)}
                        selected={s.id === activeId}
                        onActivate={(): void => {
                            setPinned(false);
                            onPick(s.id);
                        }}
                        // Symmetric side gutters so the leading grip + trailing
                        // kebab circles never hug / get clipped by the screen edge
                        // (floored at 12px, but yielding to a larger safe-area
                        // inset on the notch side in landscape — ui.md §7).
                        sx={{
                            ...(desktop && desktopListItemSx()),
                            pl: "max(env(safe-area-inset-left), 12px)",
                            pr: "max(env(safe-area-inset-right), 12px)",
                            mx: 0.75,
                            my: 0.25,
                            "@media (pointer: fine) and (hover: hover)": {
                                pl: 0.75,
                                pr: 0.5,
                                py: 0.25,
                            },
                        }}
                    >
                        {/* Leading grip — drag to reorder. A real 44px IconButton
                            keeps the Apple HIG touch target stable while the
                            1.5rem glyph follows the global font-size preference.
                            stopPropagation in handleProps keeps a row tap (select)
                            and the sheet's drag separate from a reorder. */}
                        <Box
                            sx={{
                                position: "relative",
                                width: 44,
                                height: 44,
                                flexShrink: 0,
                                display: "grid",
                                placeItems: "center",
                            }}
                        >
                            <IconButton
                                className="cowboy-session-grip"
                                {...sortable.handleProps(s.id)}
                                aria-label="Drag to reorder"
                                sx={{ width: 44, height: 44, color: "text.disabled" }}
                            >
                                <DragIndicator sx={{ fontSize: "1.5rem" }} />
                            </IconButton>
                        </Box>
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
                                    {(s.system || (s.origin ?? "api") !== "web") && (
                                        <Chip
                                            size="small"
                                            label={s.system ? "System" : "External"}
                                            color={s.system ? "secondary" : "default"}
                                            sx={{
                                                height: "1.5rem",
                                                fontSize: "0.75rem",
                                                "& .MuiChip-label": { px: "0.625rem" },
                                            }}
                                        />
                                    )}
                                    {(s.machine_id ?? "local") !== "local" && (
                                        <Chip
                                            size="small"
                                            label={s.machine_id}
                                            variant="outlined"
                                            sx={{
                                                height: "1.5rem",
                                                maxWidth: "8rem",
                                                fontSize: "0.75rem",
                                                "& .MuiChip-label": {
                                                    px: "0.625rem",
                                                    overflow: "hidden",
                                                    textOverflow: "ellipsis",
                                                },
                                            }}
                                        />
                                    )}
                                    <SessionProjectionBadge sessionId={s.id} />
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
                        {desktop && index < 10 && (
                            <Suspense fallback={null}>
                                <DesktopSessionShortcut
                                    digit={index === 9 ? "0" : String(index + 1)}
                                    active={s.id === activeId}
                                    title={s.title}
                                />
                            </Suspense>
                        )}
                        <IconButton
                            className="cowboy-session-actions"
                            aria-label={`row actions ${s.id}`}
                            onClick={(e): void => {
                                e.stopPropagation();
                                setMenuAnchor({ row: s, el: e.currentTarget });
                            }}
                            // Keep the 44px Apple-HIG tap target fixed, but let the
                            // 1.5rem glyph track the global font-size preference.
                            sx={{ ml: 0.5, width: 44, height: 44, flexShrink: 0 }}
                        >
                            <MoreVert sx={{ fontSize: "1.5rem" }} />
                        </IconButton>
                    </ReliableListItemButton>
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
            {mobileDrawer && (
                <Box
                    sx={{
                        position: "absolute",
                        zIndex: 3,
                        left: 0,
                        right: 0,
                        bottom: "max(env(safe-area-inset-bottom, 0px), 12px)",
                        display: "flex",
                        justifyContent: "center",
                        // The open foreground card keeps a deep edge shadow over
                        // the revealed rail. On iPad the rail is capped at 440px,
                        // so a centred two-action island can otherwise put its
                        // trailing Close action under that edge. Reserve the
                        // shadow/gesture seam inside the rail; phones have enough
                        // proportional reveal and retain true centring.
                        boxSizing: "border-box",
                        pr: phone ? 0 : 4,
                        pointerEvents: "none",
                    }}
                >
                    <MobileSheetActionGroup
                        actions={[
                            {
                                key: "new",
                                label: "New session",
                                onPress: onNew,
                                icon: <Add aria-hidden sx={{ fontSize: "1.35em" }} />,
                            },
                            {
                                key: "close",
                                label: "Close sessions",
                                onPress: onClose ?? (() => undefined),
                                icon: <CloseIcon aria-hidden sx={{ fontSize: "1.25em" }} />,
                            },
                        ]}
                    />
                </Box>
            )}
            <Menu
                sx={{ display: desktop ? "none" : undefined }}
                anchorEl={menuAnchor?.el ?? null}
                open={!desktop && !!menuAnchor}
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
                    View mode
                </ListSubheader>
                {(
                    [
                        { v: "history", label: "Conversation" },
                        { v: "explore", label: "Page view" },
                    ] as const
                ).map((opt) => {
                    const current = menuProjection === opt.v;
                    return (
                        <MenuItem
                            key={opt.v}
                            selected={current}
                            onClick={(): void => setMenuProjection(opt.v)}
                        >
                            <ListItemIcon>
                                {current ? <CheckIcon fontSize="medium" /> : null}
                            </ListItemIcon>
                            <ListItemText primary={opt.label} />
                        </MenuItem>
                    );
                })}
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
            {desktop && (
                <DesktopModalShell
                        open={!!menuAnchor}
                        onClose={(): void => setMenuAnchor(null)}
                        title="Session"
                        description={`${menuAnchor?.row.title ?? ""} · ${menuAnchor?.row.cwd ?? ""}`}
                        width={920}
                        shortcutGroups={[
                            {
                                label: "Navigate",
                                slots: [
                                    { shortcut: "J/K", label: "Move" },
                                    { shortcut: "Enter", label: "Select" },
                                ],
                            },
                            { slots: [{ shortcut: "Esc", label: "Close" }] },
                        ]}
                    >
                        <Box
                            onKeyDown={(event): void => {
                            if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
                            const key = workspaceCommandKey(event.nativeEvent).toLowerCase();
                            const directAction = event.currentTarget.querySelector<HTMLButtonElement>(
                                `[data-session-shortcut="${key}"]`,
                            );
                            if (directAction) {
                                event.preventDefault();
                                event.stopPropagation();
                                directAction.click();
                                return;
                            }
                            if (key !== "j" && key !== "k") return;
                            const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>(
                                "[data-desktop-session-actions] button:not(:disabled)",
                            )];
                            if (items.length === 0) return;
                            event.preventDefault();
                            event.stopPropagation();
                            const current = items.indexOf(document.activeElement as HTMLButtonElement);
                            const next = current < 0
                                ? 0
                                : Math.max(0, Math.min(items.length - 1, current + (key === "j" ? 1 : -1)));
                            items[next]?.focus();
                            }}
                            sx={{ p: 2 }}
                        >
                            <Box sx={{ display: "grid", gridTemplateColumns: "minmax(0, 1.2fr) minmax(280px, 0.8fr)", gap: 2.5 }}>
                        <DesktopSessionInfoPanel session={menuAnchor?.row ?? null} />
                        <Stack
                            data-desktop-session-actions
                            spacing={0.5}
                        >
                            <Typography variant="overline" color="text.secondary" sx={{ px: 1 }}>Actions</Typography>
                            <Button data-session-shortcut="r" autoFocus fullWidth startIcon={<DriveFileRenameOutline />} onClick={(): void => { if (menuAnchor) onRequestRename(menuAnchor.row); setMenuAnchor(null); }} sx={{ justifyContent: "flex-start" }}>
                                <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Rename</Box>
                                <Kbd keys="R" />
                            </Button>
                            <Divider sx={{ my: 0.5 }} />
                            <Typography variant="overline" color="text.secondary" sx={{ px: 1 }}>View mode</Typography>
                            {([
                                { v: "history", label: "Conversation", shortcut: "C" },
                                { v: "explore", label: "Page view", shortcut: "P" },
                            ] as const).map((opt) => {
                                const current = menuProjection === opt.v;
                                return <Button data-session-shortcut={opt.shortcut.toLowerCase()} key={opt.v} fullWidth startIcon={current ? <CheckIcon /> : <Box sx={{ width: 24 }} />} onClick={(): void => setMenuProjection(opt.v)} sx={{ justifyContent: "flex-start" }}>
                                    <Box component="span" sx={{ flex: 1, textAlign: "left" }}>{opt.label}</Box>
                                    <Kbd keys={opt.shortcut} />
                                </Button>;
                            })}
                            <Divider sx={{ my: 0.5 }} />
                            <Typography variant="overline" color="text.secondary" sx={{ px: 1 }}>Auto-resume</Typography>
                            {([ { v: null, label: `Default (${autoResumeDefault ? "on" : "off"})`, shortcut: "D" }, { v: true, label: "On", shortcut: "O" }, { v: false, label: "Off", shortcut: "F" } ] as const).map((opt) => {
                                const current = (menuAnchor?.row.auto_resume ?? null) === opt.v;
                                return <Button data-session-shortcut={opt.shortcut.toLowerCase()} key={String(opt.v)} fullWidth startIcon={current ? <CheckIcon /> : <Box sx={{ width: 24 }} />} onClick={(): void => { if (menuAnchor) setSessionAutoResume(menuAnchor.row.id, opt.v); setMenuAnchor(null); }} sx={{ justifyContent: "flex-start" }}>
                                    <Box component="span" sx={{ flex: 1, textAlign: "left" }}>{opt.label}</Box>
                                    <Kbd keys={opt.shortcut} />
                                </Button>;
                            })}
                            <Divider sx={{ my: 0.5 }} />
                            <Button data-session-shortcut="x" color="error" fullWidth startIcon={<DeleteOutline />} onClick={(): void => { if (menuAnchor) onRequestDelete(menuAnchor.row); setMenuAnchor(null); }} sx={{ justifyContent: "flex-start" }}>
                                <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Delete</Box>
                                <Kbd keys="X" />
                            </Button>
                        </Stack>
                            </Box>
                        </Box>
                </DesktopModalShell>
            )}
        </Stack>
    );
}

// Hard-coded workspace choices for v0. Each entry's `value` is what the
// daemon receives as `cwd`; the resolver in supervisor.rs honours absolute
// paths as-is and joins relative ones to `--workspace-root` (defaults to
// `/home/draven`). To expose more roots later, either bump this list or
// fetch a list from the daemon.
const WORKING_DIRS = [
    { value: "columbus", label: "columbus", help: "/home/draven/columbus", active_work_items: [] },
    { value: "/etc/nixos", label: "/etc/nixos", help: "NixOS host config", active_work_items: [] },
] as const;

type WorkspaceWorkItem = {
    id: string;
    title: string;
    projects: string[];
    recipe: string;
    blocked: boolean;
};

type WorkspaceChoice = {
    value: string;
    label: string;
    help: string;
    project?: string;
    active_work_items: readonly WorkspaceWorkItem[];
};

type MachineChoice = {
    id: string;
    display_name: string;
    platform: string;
    architecture: string;
    status: MachinePresence;
    local: boolean;
    schedulable: boolean;
    capacity: { max_sessions: number; draining: boolean };
    active_sessions: number;
    pending_updates?: readonly { kind: string; slot?: string }[];
    workspaces: readonly {
        id: string;
        display_name: string;
        canonical_path: string;
    }[];
    components: readonly {
        id: { kind: string; slot?: string };
        state: string;
        version: string;
        generation: string;
        rollback_generation?: string;
        active_leases: number;
        auth?: string;
        detail?: string;
        update?: MachineComponentUpdate;
    }[];
};

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
    const [provider, setProvider] = useState<string>("codex");
    const [machineId, setMachineId] = useState<string>("");
    const [machines, setMachines] = useState<readonly MachineChoice[]>([]);
    const desktop = useSurfaceProfile().kind === "desktop";
    const [cwd, setCwd] = useState<string>(WORKING_DIRS[0].value);
    const [workItemId, setWorkItemId] = useState<string>("");
    // Working-dir choices: start from the hard-coded fallback, then replace with
    // the daemon's `/api/workspaces` (host roots + every columbus-managed
    // project) once the dialog opens. Falling back keeps the dialog usable if
    // the endpoint is unreachable (older daemon / fetch error).
    const [workspaces, setWorkspaces] =
        useState<readonly WorkspaceChoice[]>(WORKING_DIRS);
    const selectedWorkspace = workspaces.find((workspace) => workspace.value === cwd);
    const selectedMachine = machines.find((machine) => machine.id === machineId);
    const providerAvailable = (candidate: string): boolean => Boolean(
        selectedMachine && machineProviderAvailable(candidate, selectedMachine.components),
    );
    useEffect(() => {
        if (!providerAvailable(provider)) {
            const fallback = PROVIDERS.find(providerAvailable);
            if (fallback) setProvider(fallback);
        }
    }, [machineId, machines, provider]);
    const selectedWorkItem = selectedWorkspace?.active_work_items.find(
        (item) => item.id === workItemId,
    );
    // Editable session title. Empty on Create → renameSession no-ops → the
    // daemon's default + first-prompt auto-title apply. RESET to a fresh default
    // on every open (below): the sheet stays mounted, so this state would
    // otherwise survive and show the last-typed value.
    const [title, setTitle] = useState<string>("");
    const titleRef = useRef<HTMLInputElement>(null);
    // Session count, captured in a ref so its default name is computed at open
    // time WITHOUT a session arriving mid-edit clobbering what you're typing.
    const sessionCount = useStoreSelector((snapshot) => snapshot.sessions.length);
    const sessionCountRef = useRef(sessionCount);
    sessionCountRef.current = sessionCount;
    // On open: reset to a fresh "New session N" default (N keeps it distinct if
    // you open several without renaming), then focus + select it so you can type
    // a name straight away — or clear it to let the first message auto-name.
    // autoFocus (below) is the keyboard's best shot within the opening tap's
    // gesture window (iOS only raises it for an in-gesture focus); the delayed
    // select() highlights the default once the sheet has mounted the field.
    useEffect(() => {
        if (!open) return undefined;
        setTitle(`New session ${sessionCountRef.current + 1}`);
        setProvider("codex");
        setMachineId("");
        setWorkItemId("");
        const t = globalThis.setTimeout(() => {
            titleRef.current?.focus();
            titleRef.current?.select();
        }, 60);
        return () => globalThis.clearTimeout(t);
    }, [open]);
    useEffect(() => {
        if (!open) return;
        void fetch("/api/machines")
            .then((r) => (r.ok ? r.json() : null))
            .then((data: MachineChoice[] | null) => {
                if (Array.isArray(data) && data.length > 0) {
                    setMachines(data);
                    setMachineId((current) =>
                        current || data.find((machine) => machine.local && machine.schedulable)?.id ||
                            data.find((machine) => machine.schedulable)?.id || data[0]!.id
                    );
                }
            })
            .catch(() => {
                // Older daemons remain an implicit local machine.
            });
    }, [open]);
    useEffect(() => {
        if (!open) return;
        if (machineId) {
            const machine = machines.find((candidate) => candidate.id === machineId);
            const choices: WorkspaceChoice[] = (machine?.workspaces ?? []).map((workspace) => ({
                value: workspace.id,
                label: workspace.display_name,
                help: workspace.canonical_path,
                active_work_items: [],
            }));
            setWorkspaces(choices);
            setCwd(defaultNewSessionWorkspace(choices)?.value ?? "");
            setWorkItemId("");
        }
    }, [open, machineId, machines]);
    const navbarAtBottom = useNavbarAtBottom();
    const create = (): void => {
        // POST (not the fire-and-forget WS `new_session`) so we get the assigned
        // id back synchronously and can focus the new session the moment it's
        // created.
        void fetch("/api/sessions", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ provider, machine_id: machineId, cwd, origin: "web" }),
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
                    if (selectedWorkItem) {
                        void fetch(`/api/sessions/${encodeURIComponent(data.session_id)}/prompt`, {
                            method: "POST",
                            headers: { "content-type": "application/json" },
                            body: JSON.stringify({
                                text: `Resume Columbus work item ${selectedWorkItem.id}. Read its durable metadata and relevant artifact, then set the native Codex goal. Keep plan, progress, review, and session state in Codex.`,
                            }),
                        });
                    }
                }
            })
            .catch(() => {
                // Network/daemon error surfaces via the WS error channel.
            });
        onClose();
    };
    // Desktop confirmations consistently require Mod+Enter. Keep bare Enter
    // available to the Select controls in this form; the title field and
    // focused Create button suppress their own native bare-Enter activation.
    // Mobile retains its established single-Enter form behaviour.
    useConfirmEnter(open && desktop, create, { suppressBareEnter: false });
    // BottomSheet (not a centered Dialog) to match the rest of the modals — they
    // all rise from the bottom on the mobile tier.
    return (
        <Sheet
            forceSheet={navbarAtBottom}
            open={open}
            onClose={onClose}
            title="New session"
            mobileDismiss="none"
            actions={
                <>
                    <Button onClick={onClose} color="inherit">
                        Cancel
                        <Kbd keys="Esc" />
                    </Button>
                    <Button
                        onClick={create}
                        onKeyDown={(e): void => {
                            if (
                                desktop && e.key === "Enter" &&
                                !e.metaKey && !e.ctrlKey &&
                                !isImeKeyEvent(e.nativeEvent)
                            ) e.preventDefault();
                        }}
                        variant="contained"
                    >
                        Create
                        <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
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
                        // Mobile keeps its touch-form Enter behaviour. Desktop uses the
                        // modal-wide Mod+Enter handler; bare Enter must never confirm.
                        if (
                            e.key === "Enter" && !e.shiftKey &&
                            !isImeKeyEvent(e.nativeEvent)
                        ) {
                            e.preventDefault();
                            if (!desktop) create();
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
                        <MenuItem key={p} value={p} disabled={!providerAvailable(p)}>
                            {p}
                        </MenuItem>
                    ))}
                </TextField>
                {machines.length > 1 ? (
                    <TextField
                        select
                        label="Machine"
                        value={machineId}
                        onChange={(e): void => setMachineId(e.target.value)}
                        helperText="Sessions stay on the selected machine"
                    >
                        {machines.map((machine) => (
                            <MenuItem
                                key={machine.id}
                                value={machine.id}
                                disabled={!machine.schedulable}
                            >
                                {machine.display_name}{machine.local ? " · This machine" : ""}
                            </MenuItem>
                        ))}
                    </TextField>
                ) : null}
                <TextField
                    select
                    label="Working directory"
                    value={cwd}
                    onChange={(e): void => {
                        setCwd(e.target.value);
                        setWorkItemId("");
                    }}
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
                {selectedWorkspace && selectedWorkspace.active_work_items.length > 0 ? (
                    <TextField
                        select
                        label="Durable work item"
                        value={workItemId}
                        onChange={(e): void => setWorkItemId(e.target.value)}
                        helperText="Optional · resumes durable context in a native Codex task"
                    >
                        <MenuItem value="">New task</MenuItem>
                        {selectedWorkspace.active_work_items.map((item) => (
                            <MenuItem key={item.id} value={item.id}>
                                {item.title}{item.blocked ? " · blocked" : ""}
                            </MenuItem>
                        ))}
                    </TextField>
                ) : null}
            </Stack>
        </Sheet>
    );
}

const EMPTY_TRANSCRIPT_TIMELINE: Envelope[] = [];

const StoreTranscript = memo(function StoreTranscript({
    sessionId,
    status,
    provider,
    cwd,
    topInset,
    bottomInset,
    onScrollableChange,
    desktopNavigation,
}: {
    sessionId: string;
    status: Status;
    provider: string;
    cwd: string;
    topInset: string;
    bottomInset: string;
    onScrollableChange?: ((scrollable: boolean) => void) | undefined;
    desktopNavigation: boolean;
}): React.JSX.Element {
    // Streaming changes only this boundary. Keeping the three high-frequency
    // store slices out of App prevents every ACP chunk from re-running the
    // navigation, sheets, composer shell and glass layout while preserving the
    // Transcript's established render cadence and exact visual behaviour.
    const timeline = useStoreSelector((snapshot) =>
        snapshot.timelines.get(sessionId)
    );
    const hydrated = useStoreSelector((snapshot) =>
        snapshot.hydrated.has(sessionId)
    );
    const connected = useStoreSelector((snapshot) => snapshot.connected);
    const { projection, transitionAnchorKey } = useExploreSessionState(sessionId);

    return projection === "explore" ? (
        <ExploreTranscript
            desktop={desktopNavigation}
            sessionId={sessionId}
            timeline={timeline ?? EMPTY_TRANSCRIPT_TIMELINE}
            status={status}
            provider={provider}
            cwd={cwd}
            loading={!hydrated}
            connected={connected}
            topInset={topInset}
            bottomInset={bottomInset}
            onScrollableChange={onScrollableChange}
        />
    ) : (
        <Transcript
            desktopNavigation={desktopNavigation}
            sessionId={sessionId}
            timeline={timeline ?? EMPTY_TRANSCRIPT_TIMELINE}
            status={status}
            provider={provider}
            cwd={cwd}
            loading={!hydrated}
            connected={connected}
            topInset={topInset}
            bottomInset={bottomInset}
            onScrollableChange={onScrollableChange}
            shortContentAtTop={desktopNavigation}
            restoreAnchorKey={transitionAnchorKey}
            onAnchorRestored={(): void => resolveProjectionAnchor(sessionId)}
        />
    );
});

export function App({
    themeMode,
    onSetThemeMode,
    surface,
    onMobileDrawerOpenChange,
}: {
    themeMode: ThemeMode;
    onSetThemeMode: (m: ThemeMode) => void;
    surface: "desktop" | "touch";
    onMobileDrawerOpenChange?: (open: boolean) => void;
}): React.JSX.Element {
    const sessions = useStoreSelector((snapshot) => snapshot.sessions);
    const lastError = useStoreSelector((snapshot) => snapshot.lastError);
    const sessionsLoaded = useStoreSelector((snapshot) => snapshot.sessionsLoaded);
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
    // Product identity and available space are deliberately separate axes.
    // Touch is the Mobile product; narrowing a Desktop window must never opt it
    // into Mobile's overlay composer, bottom navigation, sheets, or progressive
    // disclosure. Only the Sessions rail is allowed to collapse, below 1100px,
    // leaving the keyboard-first Prompt + Conversation workspace intact. That
    // threshold follows the useful pane floors (roughly 360 Prompt + 520
    // Conversation + rail), not a device label: Desktop spends horizontal space
    // on parallel context without squeezing the actual work panes into slivers.
    const mobile = surface === "touch";
    const phone = useMediaQuery("(max-width:767.95px) and (pointer:coarse)");
    const compactDesktopWidth = useMediaQuery("(max-width:1099px)");
    const desktopNavCollapsed = surface === "desktop" && compactDesktopWidth;
    const sessionsInDrawer = mobile || desktopNavCollapsed;
    // Navbar placement belongs exclusively to the Touch product. When its user
    // picks "bottom", the AppBar moves below the transcript, just
    // above the composer (mobile-browser bottom-bar feel). The modals read the
    // same flag and force their bottom-sheet surface (see BottomSheet
    // `forceSheet`), so a tablet's bottom navbar gets bottom-up modals too
    // rather than centered dialogs.
    const prefersNavbarAtBottom = useNavbarAtBottom();
    const navbarAtBottom = mobile && prefersNavbarAtBottom;
    const floatingPanelHeight = navbarAtBottom
        ? "calc(var(--composer-h, 0px) + var(--navbar-h, 0px))"
        : "var(--composer-h, 0px)";
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
    const activeId = useActiveSessionId();
    const setActiveId = setActiveSessionId;
    // Floating-glass inset: publish the panel's TRUE live height — the AppBar plus
    // the composer (the latter INCLUDING an expanded queue/drafts panel) — as CSS
    // vars on the column. The glass follows every animation frame, while the
    // transcript reservation is frozen during disclosure transitions: changing
    // padding-bottom on a long column-reverse transcript lays out every retained
    // row, so mirroring a 200ms Collapse transition frame-for-frame wastes the
    // mobile main thread and rate-limiting it merely turns motion into visible
    // stair-steps. One final, non-animated alignment is both cheaper and calmer.
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
    const activeDisclosureTransitionsRef = useRef(new Set<EventTarget>());
    const pendingTranscriptInsetRef = useRef("0px");
    const publishTranscriptInset = useCallback((): void => {
        const col = columnRef.current;
        if (!col) return;
        col.style.setProperty("--transcript-composer-h", pendingTranscriptInsetRef.current);
    }, []);
    const measureGlass = useCallback((): void => {
        const col = columnRef.current;
        if (!col) return;
        const navbarHeight = `${appBarElRef.current?.offsetHeight ?? 0}px`;
        const composerHeight = `${composerElRef.current?.offsetHeight ?? 0}px`;
        col.style.setProperty("--navbar-h", navbarHeight);
        col.style.setProperty("--composer-h", composerHeight);
        pendingTranscriptInsetRef.current = composerHeight;
        if (activeDisclosureTransitionsRef.current.size === 0) publishTranscriptInset();
    }, [publishTranscriptInset]);
    const disclosureTransition = useCallback((event: TransitionEvent): void => {
        const target = event.target;
        if (
            event.propertyName !== "height" ||
            !(target instanceof HTMLElement) ||
            !target.classList.contains("MuiCollapse-root")
        ) return;
        if (event.type === "transitionrun") {
            activeDisclosureTransitionsRef.current.add(target);
            return;
        }
        activeDisclosureTransitionsRef.current.delete(target);
        if (activeDisclosureTransitionsRef.current.size === 0) publishTranscriptInset();
    }, [publishTranscriptInset]);
    const observeGlass = useCallback(
        (slot: "appbar" | "composer", el: HTMLElement | null): void => {
            roRef.current ??= new ResizeObserver((): void => measureGlass());
            const ro = roRef.current;
            const prev = slot === "appbar" ? appBarElRef.current : composerElRef.current;
            if (prev) ro.unobserve(prev);
            if (slot === "composer" && prev) {
                prev.removeEventListener("transitionrun", disclosureTransition, true);
                prev.removeEventListener("transitionend", disclosureTransition, true);
                prev.removeEventListener("transitioncancel", disclosureTransition, true);
                activeDisclosureTransitionsRef.current.clear();
            }
            if (slot === "appbar") appBarElRef.current = el;
            else composerElRef.current = el;
            if (el) {
                ro.observe(el);
                if (slot === "composer") {
                    el.addEventListener("transitionrun", disclosureTransition, true);
                    el.addEventListener("transitionend", disclosureTransition, true);
                    el.addEventListener("transitioncancel", disclosureTransition, true);
                }
            }
            measureGlass();
        },
        [disclosureTransition, measureGlass],
    );
    const appBarRef = useCallback(
        (el: HTMLDivElement | null): void => observeGlass("appbar", el),
        [observeGlass],
    );
    const composerRef = useCallback(
        (el: HTMLDivElement | null): void => observeGlass("composer", el),
        [observeGlass],
    );
    // Disconnect the shared observer and transition listeners on unmount.
    useEffect(() => (): void => {
        roRef.current?.disconnect();
        const composer = composerElRef.current;
        if (composer) {
            composer.removeEventListener("transitionrun", disclosureTransition, true);
            composer.removeEventListener("transitionend", disclosureTransition, true);
            composer.removeEventListener("transitioncancel", disclosureTransition, true);
        }
    }, [disclosureTransition]);
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
    const restoredFocusRef = useRef<string | null>(getActiveSessionId());
    const goneCheckedRef = useRef(false);
    const [drawerOpen, setDrawerOpen] = useState(false);
    const drawerOpenRef = useRef(false);
    useEffect(() => {
        if (!mobile) return undefined;
        onMobileDrawerOpenChange?.(drawerOpen);
        return () => onMobileDrawerOpenChange?.(false);
    }, [drawerOpen, mobile, onMobileDrawerOpenChange]);
    const mobileShellRef = useRef<HTMLDivElement>(null);
    const mobileDrawerRef = useRef<HTMLDivElement>(null);
    const mobileDrawerMaskRef = useRef<HTMLDivElement>(null);
    const settleMobileDrawerRef = useRef<(
        (
            open: boolean,
            releaseVelocity?: number,
            onSettled?: () => void,
            cachedWidth?: number,
        ) => void
    ) | null>(null);
    const [dialogOpen, setDialogOpen] = useState(false);
    const [settingsOpen, setSettingsOpen] = useState(false);
    const [exploreComposeIntent, setExploreComposeIntent] = useState<{
        kind: "follow_up" | "new_page";
        targetPageId: string | null;
        knownPageIds: string[];
    } | null>(null);

    // Mobile Sessions and Code Review use the same spatial-drawer controller.
    // Product code supplies only the side and presentation-freeze hook; sampling,
    // prediction, magnetic thresholds, haptics, rubber-band, depth, settle timing,
    // and idle release stay identical on both surfaces.
    useEffect(() => {
        if (!mobile || anySheetOpen) return undefined;
        const surface = columnRef.current;
        const gestureTarget = mobileShellRef.current;
        const drawer = mobileDrawerRef.current;
        const drawerMask = mobileDrawerMaskRef.current;
        if (!surface || !gestureTarget || !drawer || !drawerMask) return undefined;
        const binding = bindMobileSpatialDrawer({
            gestureTarget,
            surface,
            drawer,
            drawerMask,
            side: "left",
            phone,
            getOpen: () => drawerOpenRef.current,
            setOpen: (open) => {
                drawerOpenRef.current = open;
                setDrawerOpen(open);
            },
            holdPresentation: holdStorePresentation,
        });
        settleMobileDrawerRef.current = binding.settle;
        return () => {
            settleMobileDrawerRef.current = null;
            binding.dispose();
        };
    }, [anySheetOpen, mobile, phone]);

    // The native viewport owns full-screen device corners. While the keyboard
    // is present, temporarily square the drawer surface's bottom edge; on
    // release, remove only that override so an open drawer can recover its
    // shared card radius and a closed drawer remains entirely native-clipped.
    useEffect(() => {
        if (!mobile) return undefined;
        const surface = columnRef.current;
        if (!surface) return undefined;
        let restoreTimer = 0;
        const ownsKeyboard = (): boolean => {
            const active = document.activeElement;
            return active instanceof HTMLElement &&
                active.matches("input, textarea, [contenteditable='true']");
        };
        const apply = (): void => {
            if (ownsKeyboard()) {
                surface.style.borderBottomLeftRadius = "0px";
                surface.style.borderBottomRightRadius = "0px";
            } else {
                surface.style.removeProperty("border-bottom-left-radius");
                surface.style.removeProperty("border-bottom-right-radius");
            }
        };
        const onFocusIn = (): void => {
            globalThis.clearTimeout(restoreTimer);
            apply();
        };
        const onFocusOut = (): void => {
            globalThis.clearTimeout(restoreTimer);
            restoreTimer = globalThis.setTimeout(apply, 420);
        };
        document.addEventListener("focusin", onFocusIn);
        document.addEventListener("focusout", onFocusOut);
        apply();
        return () => {
            document.removeEventListener("focusin", onFocusIn);
            document.removeEventListener("focusout", onFocusOut);
            globalThis.clearTimeout(restoreTimer);
            surface.style.removeProperty("border-bottom-left-radius");
            surface.style.removeProperty("border-bottom-right-radius");
        };
    }, [mobile]);

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
    // Two-pane working area (plus the Sessions rail when space permits). Desktop
    // remains split even when the rail collapses: a narrower desktop is still a
    // keyboard-first production surface, not Mobile. Mobile never reads this
    // preference and always owns its separate overlay composer path.
    const splitActive = surface === "desktop";
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
    const exploreState = useExploreSessionState(active?.id ?? "__none__");
    const changeTranscriptProjection = useCallback((
        sessionId: string,
        projection: "history" | "explore",
    ): void => {
        const anchor = captureTranscriptViewportAnchor(sessionId);
        setTranscriptProjection(sessionId, projection, anchor);
    }, []);
    useEffect(() => {
        setExploreComposeIntent(null);
    }, [active?.id, exploreState.projection]);
    const previousActiveRef = useRef<string | null>(null);
    useEffect(() => {
        const previous = previousActiveRef.current;
        if (previous && previous !== active?.id) releaseInactiveHistory(previous);
        previousActiveRef.current = active?.id ?? null;
    }, [active?.id]);
    useEffect(() => {
        if (!sessionsLoaded) return;
        retainTranscriptViewportSessions(new Set(sessions.map((session) => session.id)));
    }, [sessions, sessionsLoaded]);

    // Persist the *resolved* focus so a reload reopens it. Keyed on `active.id`
    // (not raw `activeId`) so a stale stored id that fell back to sessions[0]
    // gets corrected to what's actually shown. Skip while `active` is null —
    // during the initial load (sessions not yet broadcast) we must not clobber
    // the stored id with null before it has a chance to resolve.
    useEffect(() => {
        if (active) {
            setActiveSessionId(active.id);
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
        globalThis.dispatchEvent(
            new CustomEvent("cowboy:transcript-save-viewport"),
        );
        if (mobile && settleMobileDrawerRef.current) {
            // Keep the current transcript raster stable during close. Switching
            // the large active timeline mid-transform was the principal fast-
            // swipe frame drop and visible content flash.
            settleMobileDrawerRef.current(false, 0, () => {
                requestAnimationFrame(() => {
                    startTransition(() => setActiveId(id));
                });
            });
        } else {
            setActiveId(id);
            setDrawerOpen(false);
        }
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
            onClose={mobile
                ? (): void => settleMobileDrawerRef.current?.(false)
                : undefined}
            onRequestDelete={(s): void => setPendingDelete(s)}
            onRequestInfo={(s): void => setPendingInfo(s)}
            onRequestRename={(s): void => {
                claimKeyboard(); // raise the keyboard in-gesture (iOS)
                setPendingRename(s);
            }}
            autoResumeDefault={autoResumeDefaultOn}
            loaded={sessionsLoaded}
            desktop={surface === "desktop"}
            mobileDrawer={mobile}
            mobileDrawerOpen={mobile && drawerOpen}
            phone={phone}
        />
    );

    // Brand toolbar for the desktop sidebar. Uses MUI's own `<Toolbar>` so
    // its height comes from `theme.mixins.toolbar` (responsive across
    // breakpoints) and stays in lockstep with the AppBar's Toolbar on the
    // right pane — no hardcoded pixel value. On mobile the sidebar is a
    // Drawer and the brand goes in the AppBar instead.
    const sidebarHeader = (
        <Toolbar
            data-desktop-pane-header
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
            <Typography
                variant="overline"
                sx={{
                    color: "text.secondary",
                    fontWeight: 700,
                    letterSpacing: "0.09em",
                    lineHeight: 1,
                    WebkitAppRegion: "no-drag",
                }}
            >
                Sessions
            </Typography>
            <Box sx={{ flex: 1 }} />
            {surface === "desktop" && (
                <Suspense fallback={null}>
                    <DesktopRegionShortcut
                        shortcut="Mod+E"
                        title="Focus Sessions"
                        singleKeycap={`${MOD_LABEL}E`}
                    />
                </Suspense>
            )}
        </Toolbar>
    );

    return (
        <Box
            sx={{
                display: "flex",
                flexDirection: "column",
                height: "100%",
                width: "100%",
                ...(surface === "desktop" && {
                    "& [data-desktop-region]": {
                        position: "relative",
                        outline: "none",
                        transition: "background-color 120ms ease, border-color 120ms ease, box-shadow 120ms ease",
                    },
                    "& [data-desktop-region][data-desktop-focused='true']:not([data-desktop-region='prompt.plan']):not([data-desktop-region='prompt.queued']):not([data-desktop-region='prompt.draft']):not([data-desktop-region='prompt.composer']):not([data-desktop-region='topbar.controls']):not([data-desktop-region='conversation.transcript']):not([data-desktop-region='sessions.list'])": {
                        bgcolor: desktopFocusFill,
                        boxShadow: (t) =>
                            `inset 0 0 0 1px ${desktopFocusBoundary(t)}`,
                    },
                    // Regions with a real outlined surface recolor that existing
                    // 1px edge instead of stacking an inset boundary on top.
                    "& [data-desktop-region='prompt.queued'][data-desktop-focused='true'], & [data-desktop-region='prompt.draft'][data-desktop-focused='true']": {
                        borderColor: desktopFocusBoundary,
                        bgcolor: desktopFocusFill,
                        boxShadow: "none",
                    },
                    // Plan's region wrapper has no visual geometry of its own.
                    // Recolor the real rounded PlanDock surface, exactly like
                    // Queue/Draft, instead of drawing a square ring around its
                    // margin box.
                    "& [data-desktop-region='prompt.plan'][data-desktop-focused='true'] > [data-desktop-plan-surface]": {
                        borderColor: desktopFocusBoundary,
                        bgcolor: desktopFocusFill,
                    },
                    // Topbar focus belongs to the whole Toolbar, not the inner
                    // controls Stack. Use the same header language as Prompt,
                    // Conversation and Sessions: quiet tint + a 2px accent rail.
                    "& [data-desktop-region='topbar.controls'][data-desktop-focused='true']": {
                        color: "primary.main",
                        bgcolor: desktopFocusFill,
                        boxShadow: "inset 0 2px 0 0 currentColor",
                    },
                    "& [data-desktop-pane] > [data-desktop-pane-header]": {
                        color: "text.secondary",
                        bgcolor: (t) => alpha(t.palette.background.paper, 0.16),
                        boxShadow: "none",
                    },
                    "& [data-desktop-pane][data-desktop-pane-focused='true'] > [data-desktop-pane-header]": {
                        color: "primary.main",
                        bgcolor: (t) => alpha(t.palette.primary.main, 0.08),
                        boxShadow: "inset 0 2px 0 0 currentColor",
                    },
                    // The full-height Composer expresses focus through its live
                    // caret, the Prompt header and the Desktop status line. Do
                    // not tint or ring the entire writing canvas: that treatment
                    // belongs to bounded cards such as Plan, Queue and Draft.
                    "& [data-desktop-region='prompt.composer']:focus-visible": {
                        outline: "none",
                        boxShadow: "none",
                    },
                    "& [data-desktop-item]:focus-visible": {
                        outline: "none",
                        bgcolor: (t) => alpha(t.palette.primary.main, 0.105),
                        boxShadow: (t) =>
                            `inset 0 0 0 1px ${alpha(t.palette.primary.main, 0.34)}`,
                    },
                    // Session selection and keyboard focus are distinct: the
                    // filled row is the open session, while j/k moves a keyboard
                    // cursor that l/Enter can open. MUI's :focus-visible flag is
                    // not retained reliably across programmatic focus() on
                    // macOS, so draw the cursor from real DOM :focus whenever
                    // the Sessions region owns focus.
                    "& [data-desktop-region='sessions.list'][data-desktop-focused='true'] [data-desktop-item]:focus": {
                        outline: "none",
                        borderColor: (t) => alpha(t.palette.primary.main, 0.62),
                        bgcolor: "transparent",
                        boxShadow: (t) =>
                            `0 0 0 2px ${alpha(t.palette.primary.main, 0.14)}`,
                        "& .cowboy-session-grip": {
                            color: "primary.main",
                        },
                        "& .cowboy-session-shortcut": {
                            opacity: "1 !important",
                        },
                    },
                    // The open session is persistent state; the crisp outline is
                    // only the keyboard cursor. A quiet persistent tint keeps the
                    // current session identifiable without the heavy selection
                    // rail that competed with the row border and status icon.
                    "& [data-desktop-region='sessions.list'] [data-desktop-item][data-desktop-current='true']": {
                        borderColor: (t) => alpha(t.palette.primary.main, 0.28),
                        bgcolor: (t) => alpha(t.palette.primary.main, 0.09),
                        boxShadow: "none",
                    },
                    "& [data-desktop-region='sessions.list'][data-desktop-focused='true'] [data-desktop-item][data-desktop-current='true']:focus": {
                        borderColor: (t) => alpha(t.palette.primary.main, 0.68),
                        bgcolor: (t) => alpha(t.palette.primary.main, 0.11),
                        boxShadow: (t) =>
                            `0 0 0 2px ${alpha(t.palette.primary.main, 0.15)}`,
                    },
                    "& [data-desktop-region='sessions.list'][data-desktop-pinned='true'] [data-desktop-item][data-desktop-pin-active='true']:focus": {
                        bgcolor: (t) => alpha(t.palette.primary.main, 0.105),
                        boxShadow: (t) =>
                            `inset 0 0 0 2px ${alpha(t.palette.primary.main, 0.48)}, 0 3px 14px ${alpha(t.palette.primary.main, 0.09)}`,
                    },
                }),
            }}
        >
            {/* The connection state is shared; activation policy is not.
                Desktop keeps the short update countdown, while touch surfaces
                require an explicit Update tap so foreground checks never
                replace active mobile work. */}
            {surface === "desktop" && (
                <>
                    <ConnectionBanner store={controlPlaneConnection} />
                    <NativeReleaseUpdatePrompt
                        appId="top.thundersparrow.cowboy"
                        manifestUrl="/native-release.json"
                    />
                </>
            )}
            <Box
                ref={mobileShellRef}
                sx={{
                    display: "flex",
                    flex: 1,
                    minHeight: 0,
                    width: "100%",
                    position: "relative",
                    overflow: mobile ? "hidden" : undefined,
                    // Store notifications are already held during a drawer
                    // gesture. Pause independent CSS spinners/shimmers too:
                    // otherwise they keep consuming compositor time underneath
                    // the frozen foreground raster on busy sessions.
                    "&[data-mobile-drawer-moving='true'] *": {
                        animationPlayState: "paused !important",
                    },
                }}
            >
            {surface === "desktop" && (
                <Suspense fallback={null}>
                    <DesktopCommandHost
                        onNewSession={(): void => setDialogOpen(true)}
                        onOpenSettings={(): void => openSettings("settings")}
                    />
                </Suspense>
            )}
            {mobile && (
                <Box
                    ref={mobileDrawerRef}
                    aria-hidden={!drawerOpen}
                    sx={{
                        position: "absolute",
                        zIndex: 0,
                        inset: 0,
                        overflow: "hidden",
                        bgcolor: "background.default",
                        backfaceVisibility: "hidden",
                    }}
                >
                    <Stack
                        role="navigation"
                        aria-label="Sessions"
                        sx={{
                            width: "min(84%, 360px)",
                            height: "100%",
                            minWidth: 0,
                            overflow: "hidden",
                            pt: "env(safe-area-inset-top, 0px)",
                            pl: "env(safe-area-inset-left, 0px)",
                            "@media (min-width: 768px)": {
                                width: "min(52%, 440px)",
                            },
                        }}
                    >
                        <Box sx={{ flex: 1, minHeight: 0 }}>
                            {list}
                        </Box>
                    </Stack>
                </Box>
            )}
            {mobile && (
                <Box
                    ref={mobileDrawerMaskRef}
                    aria-hidden="true"
                    sx={{
                        position: "absolute",
                        zIndex: 0,
                        inset: 0,
                        bgcolor: "background.default",
                        pointerEvents: "none",
                        backfaceVisibility: "hidden",
                    }}
                />
            )}
            {sessionsInDrawer && !mobile ? (
                // The shared momentum sheet presents the Sessions rail only when
                // it is out of flow. Touch follows its configurable navbar anchor;
                // compact Desktop always opens from the top and otherwise retains
                // the Desktop workspace, dialogs, and keyboard behavior.
                <DetentSheet
                    open={drawerOpen}
                    onClose={(): void => setDrawerOpen(false)}
                    anchor={navbarAtBottom ? "bottom" : "top"}
                    ariaLabel="Sessions"
                    // Mobile uses a full-screen frosted workspace so long session
                    // lists never compete with the transcript behind the sheet.
                    // Compact Desktop retains its top-anchored content sizing.
                    frosted
                    cover={navbarAtBottom}
                    surfaceColor={theme.palette.background.default}
                    footer={navbarAtBottom
                        ? <MobileSheetDismiss onClose={(): void => setDrawerOpen(false)} />
                        : undefined}
                    footerOverlay={navbarAtBottom}
                >
                    {/* DetentSheet's body has no side padding, so the list spans
                        the full width on its own — render it directly. (A former
                        `mx: -2` here "cancelled" a px:2 the sheet no longer has,
                        so it just bled the rows 16px PAST the viewport, clipping
                        the grip/kebab circles at the screen edge. The row's own
                        px gutter below insets the controls instead.) */}
                    {list}
                </DetentSheet>
            ) : !mobile ? (
                <Stack
                    data-desktop-pane="sessions"
                    data-desktop-region="sessions.list"
                    data-desktop-reorderable="true"
                    data-desktop-focus-default
                    tabIndex={-1}
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
            ) : null}

            <Stack
                ref={columnRef}
                data-mobile-session-surface={mobile ? "true" : undefined}
                sx={{
                    flex: 1,
                    height: mobile ? "100%" : undefined,
                    minWidth: 0,
                    // Anchor the floating frosted navbar + composer overlays (below)
                    // and clip them to the column. The overlay design (transcript is
                    // the full-height background, bars float over it as frosted glass)
                    // now applies in BOTH modes — desktop top-navbar included — so the
                    // clip is unconditional.
                    position: "relative",
                    overflow: "hidden",
                    zIndex: mobile ? 1 : undefined,
                    bgcolor: "background.default",
                    // Keep the revealed seam exactly under the finger while the
                    // frozen workspace recedes as one composited layer.
                    transformOrigin: "left center",
                    backfaceVisibility: "hidden",
                    contain: mobile ? "paint" : undefined,
                    willChange: mobile ? "transform" : undefined,
                    // Lift the whole column off the on-screen keyboard + its
                    // iOS-native accessory bar: this padding (the keyboard's
                    // overlap, published by useKeyboardInset) reserves space at
                    // the bottom, so the flex:1 transcript shrinks and the bottom
                    // group (composer, or the navbar in bottom mode) rises clear
                    // of the keyboard. 0 when no keyboard.
                    pb: "var(--kb-inset, 0px)",
                }}
            >
                {mobile && (
                    <Box
                        role="button"
                        tabIndex={drawerOpen ? 0 : -1}
                        aria-label="Close sessions"
                        aria-hidden={!drawerOpen}
                        onClick={(): void => {
                            if (drawerOpen) settleMobileDrawerRef.current?.(false);
                        }}
                        onKeyDown={(event): void => {
                            if (event.key === "Enter" || event.key === " ") {
                                event.preventDefault();
                                settleMobileDrawerRef.current?.(false);
                            }
                        }}
                        sx={{
                            position: "absolute",
                            inset: 0,
                            zIndex: (t) => t.zIndex.modal - 1,
                            // Keep this hit layer mounted across close so WebKit
                            // does not rebuild the foreground stacking tree on
                            // the animation's final frame. It is intentionally
                            // transparent; depth comes from the stable edge.
                            bgcolor: "transparent",
                            pointerEvents: drawerOpen ? "auto" : "none",
                            cursor: drawerOpen ? "pointer" : "default",
                        }}
                    />
                )}
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
                            height: floatingPanelHeight,
                            zIndex: 1,
                            pointerEvents: "none",
                            // Hide under an open cover sheet — its own frosted surface
                            // replaces this chrome; leaving it on double-frosts.
                            opacity: anySheetOpen ? 0 : 1,
                            transition: "opacity 200ms ease",
                            // Milkier than a clear pane + heavy blur + saturate → thick
                            // iOS frosted material; content scrolling under it diffuses
                            // (not shows) through the blur. Up-shadow + top hairline give
                            // the "floating above the scroll" depth, now on the slab.
                            // Saturated image bubbles remain visually loud through
                            // a thin, highly saturated glass layer and can look as
                            // if they paint above the composer. Keep the transcript
                            // physically underneath, but make this interaction
                            // surface materially opaque and reduce colour lift.
                            bgcolor: (t) =>
                                alpha(
                                    t.palette.background.default,
                                    t.palette.mode === "dark" ? 0.86 : 0.92,
                                ),
                            backdropFilter: "blur(36px) saturate(125%)",
                            WebkitBackdropFilter:
                                "blur(36px) saturate(125%)",
                    }}
                />
                )}
                {/* Keep the glass edge out of the backdrop-filter layer. WebKit
                    invalidates filtered ancestors whenever its native caret
                    blinks; painting the hairline + shadow on that same layer made
                    the whole composer edge pulse with the caret. This independent
                    composited strip stays rasterized while the editor repaints. */}
                {!splitActive && (
                <Box
                    aria-hidden
                    sx={{
                        position: "absolute",
                        left: 0,
                        right: 0,
                        bottom: `calc(var(--kb-inset, 0px) + ${floatingPanelHeight})`,
                        height: "1px",
                        zIndex: 1,
                        pointerEvents: "none",
                        opacity: anySheetOpen ? 0 : 1,
                        transition: "opacity 200ms ease",
                        bgcolor: "divider",
                        transform: "translateZ(0)",
                        backfaceVisibility: "hidden",
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
            {...(surface === "desktop"
                ? {
                    "data-desktop-region": "topbar.controls",
                    "data-desktop-axis": "horizontal",
                    tabIndex: -1,
                }
                : {})}
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
                            ...(sessionsInDrawer && {
                                "@media (display-mode: window-controls-overlay)": {
                                    pl: "calc(env(titlebar-area-x, 0px) + 12px)",
                                },
                            }),
                        }}
                    >
                        {/* Whenever the Sessions rail is hidden, its drawer toggle
                            leads the bar. This does not change the product surface. */}
                        {sessionsInDrawer && (
                            // No `edge="start"`: its negative left margin pulled the
                            // hamburger tight to the screen edge, while Settings (no
                            // edge="end") sits at the Toolbar's normal gutter — so the
                            // bar read lopsided. Dropping it gives the hamburger the
                            // same gutter, symmetric with the gear on the right.
                            <IconButton
                                aria-label={drawerOpen ? "Close sessions" : "Open sessions"}
                                onClick={(): void => {
                                    if (mobile) settleMobileDrawerRef.current?.(!drawerOpen);
                                    else setDrawerOpen(true);
                                }}
                                onPointerDown={(event): void => {
                                    if (event.pointerType === "touch") {
                                        event.currentTarget.dataset.touchActivated = "true";
                                    }
                                }}
                                onPointerEnter={(event): void => {
                                    if (event.pointerType === "mouse") {
                                        delete event.currentTarget.dataset.touchActivated;
                                    }
                                }}
                                // Unified 44px box + fixed 24px glyph (global
                                // MuiIconButton) keeps the hamburger aligned with
                                // the slash button below it at any font scale.
                                sx={{
                                    mr: 1,
                                    // iPad can advertise hover because a trackpad is
                                    // connected, then leave :hover latched after a
                                    // finger tap. Remember touch activation until a
                                    // real mouse enters, so ordinary content taps do
                                    // not leave this button looking selected.
                                    "&[data-touch-activated='true']:hover": {
                                        bgcolor: "transparent",
                                    },
                                    "&[data-touch-activated='true']:active": {
                                        bgcolor: "action.selected",
                                    },
                                }}
                            >
                                {drawerOpen ? <CloseIcon /> : <MenuIcon />}
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
                                sx={{
                                    minWidth: 0,
                                    // Desktop gives the title only the width its
                                    // text needs (up to the cap). The old 280px
                                    // flex basis left a large dead zone after a
                                    // short title while squeezing the controls.
                                    flex: surface === "desktop" ? "0 1 auto" : 1,
                                    maxWidth: surface === "desktop" ? 320 : "none",
                                }}
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
                        {surface === "desktop" ? (
                            // The complete trailing toolbar owns horizontal
                            // overflow. Its children retain their calculated
                            // minimum width; narrow split panes scroll this one
                            // strip instead of crushing quota/config buttons.
                            <Box
                                sx={{
                                    flex: 1,
                                    minWidth: 0,
                                    overflowX: "auto",
                                    overflowY: "hidden",
                                    scrollbarWidth: "thin",
                                    "&::-webkit-scrollbar": { height: 4 },
                                    "&::-webkit-scrollbar-thumb": {
                                        bgcolor: "action.disabled",
                                        borderRadius: 99,
                                    },
                                }}
                            >
                                <Stack
                                    direction="row"
                                    alignItems="center"
                                    sx={{ width: "max-content", minWidth: "100%" }}
                                >
                                    {active && (
                                        <Suspense fallback={null}>
                                            <DesktopTopBarControls
                                                sessionId={active.id}
                                                status={active.status}
                                            />
                                        </Suspense>
                                    )}
                                    <Suspense fallback={null}>
                                        <DesktopRegionShortcut
                                            shortcut="Mod+T"
                                            title="Focus Top Bar"
                                            singleKeycap={`${MOD_LABEL}T`}
                                            sx={{ mx: 0.5 }}
                                        />
                                    </Suspense>
                                    <Divider orientation="vertical" flexItem sx={{ mx: 0.75, my: 0.75 }} />
                                    <Suspense fallback={null}>
                                        <DesktopContextShortcut
                                            badge={`${MOD_LABEL},`}
                                            shortcut={`${MOD_LABEL}, · Settings`}
                                            placement="inline"
                                            alwaysVisible
                                            active={settingsOpen}
                                        >
                                            <IconButton
                                                data-desktop-item="topbar-settings"
                                                data-desktop-topbar-action="settings"
                                                onClick={(): void => openSettings("settings")}
                                                aria-label="settings"
                                                title="Settings"
                                            >
                                                <SettingsIcon />
                                            </IconButton>
                                        </DesktopContextShortcut>
                                    </Suspense>
                                </Stack>
                            </Box>
                        ) : (
                            <>
                                {active && (
                                    <SessionControls
                                        sessionId={active.id}
                                        status={active.status}
                                        projection={exploreState.projection}
                                        onProjectionChange={(projection): void =>
                                            changeTranscriptProjection(active.id, projection)}
                                    />
                                )}
                                <IconButton
                                    onClick={(): void => openSettings("settings")}
                                    aria-label="settings"
                                    title="Settings"
                                >
                                    <SettingsIcon />
                                </IconButton>
                            </>
                        )}
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
                            <Suspense fallback={null}><DesktopWorkspace
                                promptWidth={colWidth}
                                resizing={colResizing}
                                onResizeStart={startColResize}
                                sessionId={active.id}
                                projection={exploreState.projection}
                                onProjectionChange={(projection): void =>
                                    changeTranscriptProjection(active.id, projection)}
                                prompt={active.system ? (
                                    <Box sx={{ p: 1.5, textAlign: "center", fontSize: 13, opacity: 0.6 }}>
                                        View-only system session — managed by cowboy
                                    </Box>
                                ) : (
                                    <Suspense fallback={null}><DesktopComposer
                                        key={active.id}
                                        sessionId={active.id}
                                        status={active.status}
                                        variant="column"
                                    /></Suspense>
                                )}
                                conversation={(
                                    <StoreTranscript
                                        desktopNavigation
                                        sessionId={active.id}
                                        status={active.status}
                                        provider={active.provider}
                                        cwd={active.cwd}
                                        topInset="0px"
                                        bottomInset="0px"
                                    />
                                )}
                            /></Suspense>
                            <Suspense fallback={null}>
                                <DesktopStatusLine sessionId={active.id} status={active.status} />
                            </Suspense>
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
                            <StoreTranscript
                                desktopNavigation={false}
                                sessionId={active.id}
                                status={active.status}
                                provider={active.provider}
                                cwd={active.cwd}
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
                                        ? "calc(var(--transcript-composer-h, 0px) + var(--navbar-h, 0px))"
                                        : "var(--transcript-composer-h, 0px)"
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
                                    View-only system session — managed by cowboy
                                </Box>
                            ) : exploreState.projection === "explore" ? (
                                <>
                                    {exploreComposeIntent !== null && (
                                    <>
                                        <MobileComposer
                                            key={active.id}
                                            sessionId={active.id}
                                            status={active.status}
                                            autoFocus
                                            onSubmitted={(): void => {
                                                if (
                                                    exploreComposeIntent?.kind === "follow_up" &&
                                                    exploreComposeIntent.targetPageId
                                                ) {
                                                    queueExploreFollowUp(
                                                        active.id,
                                                        exploreComposeIntent.targetPageId,
                                                        exploreComposeIntent.knownPageIds,
                                                    );
                                                }
                                                setExploreComposeIntent(null);
                                            }}
                                        />
                                    </>
                                    )}
                                    <MobilePageDock
                                        sessionId={active.id}
                                        composeOpen={exploreComposeIntent !== null}
                                        onComposeToggle={(knownPageIds): void => {
                                            if (exploreComposeIntent !== null) {
                                                setExploreComposeIntent(null);
                                                return;
                                            }
                                            // Claim the iOS keyboard before mounting
                                            // the conditional composer, then let its
                                            // autoFocus transfer focus to CM6.
                                            claimKeyboard();
                                            setExploreComposeIntent({
                                                kind: "new_page",
                                                targetPageId: null,
                                                knownPageIds,
                                            });
                                        }}
                                    />
                                </>
                            ) : (
                                <MobileComposer
                                    key={active.id}
                                    sessionId={active.id}
                                    status={active.status}
                                />
                            )}
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
                                            {surface === "desktop" && <Kbd keys={`${MOD_LABEL}N`} variant="global" />}
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
                    if (mobile && settleMobileDrawerRef.current) {
                        settleMobileDrawerRef.current(false, 0, () => {
                            requestAnimationFrame(() => {
                                startTransition(() => setActiveId(id));
                            });
                        });
                    } else {
                        setActiveId(id);
                        setDrawerOpen(false);
                    }
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

function DesktopSettingsChoice({
    active,
    children,
    onClick,
    ariaLabel,
}: {
    active: boolean;
    children: React.ReactNode;
    onClick: () => void;
    ariaLabel: string;
}): React.JSX.Element {
    return (
        <ButtonBase
            data-settings-choice
            aria-label={ariaLabel}
            aria-pressed={active}
            onClick={onClick}
            sx={{
                minHeight: 38,
                minWidth: 0,
                width: "100%",
                px: 0.75,
                borderRadius: 1.25,
                border: 1,
                borderColor: active ? "primary.main" : "divider",
                bgcolor: active ? "action.selected" : "transparent",
                color: active ? "primary.main" : "text.primary",
                fontSize: "0.8125rem",
                fontWeight: active ? 700 : 500,
                justifyContent: "center",
                overflow: "hidden",
                transition: "background-color 120ms ease, border-color 120ms ease",
                "&:hover": { bgcolor: "action.hover", borderColor: "primary.light" },
                "&:focus-visible": {
                    outline: (theme) => `2px solid ${desktopFocusBoundary(theme)}`,
                    outlineOffset: 2,
                },
            }}
        >
            {children}
        </ButtonBase>
    );
}

function DesktopSettingsRow({
    shortcut,
    shortcutAvailable = true,
    label,
    description,
    children,
}: {
    shortcut: string;
    shortcutAvailable?: boolean;
    label: string;
    description: string;
    children: React.ReactNode;
}): React.JSX.Element {
    return (
        <Box
            data-settings-row
            data-settings-shortcut={shortcut.toLowerCase()}
            sx={{
                display: "grid",
                gridTemplateColumns: "minmax(116px, 0.55fr) minmax(0, 1.45fr)",
                gap: 1,
                alignItems: "center",
                minHeight: 64,
                px: 1.5,
                py: 1,
                borderRadius: 1.5,
                border: 1,
                borderColor: "transparent",
                "&:has([data-settings-choice]:focus-visible)": {
                    borderColor: "primary.main",
                    bgcolor: "action.hover",
                },
            }}
        >
            <Stack direction="row" spacing={1} alignItems="flex-start" sx={{ minWidth: 0 }}>
                <Kbd
                    keys={shortcut.toUpperCase()}
                    availability={shortcutAvailable ? "available" : "inactive"}
                />
                <Box sx={{ minWidth: 0 }}>
                    <Typography variant="body2" fontWeight={700}>{label}</Typography>
                    <Typography variant="caption" color="text.secondary">{description}</Typography>
                </Box>
            </Stack>
            {children}
        </Box>
    );
}

// Desktop modal primitive: a dense, keyboard-addressable block with one visual
// boundary, an optional mnemonic, and a stable landmark for jump navigation.
// Desktop settings, information, and logs deliberately share this shape so a
// modal reads as one workbench rather than a collection of unrelated sheets.
function DesktopModalBlock({
    label,
    title,
    shortcut,
    shortcutAvailable = true,
    section,
    children,
    sx,
}: {
    label: string;
    title?: string;
    shortcut?: string;
    shortcutAvailable?: boolean;
    section?: "machines" | "info" | "logs";
    children: React.ReactNode;
    sx?: SxProps<Theme>;
}): React.JSX.Element {
    const focusable = section !== undefined;
    return (
        <Box
            {...(section === "machines" ? { "data-settings-machines": true } : {})}
            {...(section === "info" ? { "data-settings-info": true } : {})}
            {...(section === "logs" ? { "data-settings-logs": true } : {})}
            tabIndex={focusable ? 0 : undefined}
            role={focusable ? "region" : undefined}
            aria-label={focusable ? `${label} ${title ?? ""}`.trim() : undefined}
            sx={[
                {
                    border: 1,
                    borderColor: "divider",
                    borderRadius: 2,
                    p: section ? 2 : 1,
                    scrollMarginTop: 76,
                    "&:focus-visible": {
                        outline: (theme) => `2px solid ${desktopFocusBoundary(theme)}`,
                        outlineOffset: 2,
                    },
                },
                ...(Array.isArray(sx) ? sx : sx ? [sx] : []),
            ]}
        >
            <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ px: section ? 0 : 1.5, mb: title ? 1 : 0 }}>
                <Box>
                    <Typography variant="overline" color="text.secondary">{label}</Typography>
                    {title && <Typography variant="body2" fontWeight={750}>{title}</Typography>}
                </Box>
                {shortcut && (
                    <Kbd
                        keys={shortcut}
                        availability={shortcutAvailable ? "available" : "inactive"}
                    />
                )}
            </Stack>
            {children}
        </Box>
    );
}

function DesktopSettingsContent({
    themeMode,
    onSetThemeMode,
    shortcutsAvailable,
}: {
    themeMode: ThemeMode;
    onSetThemeMode: (mode: ThemeMode) => void;
    shortcutsAvailable: boolean;
}): React.JSX.Element {
    const vim = useVimSetting();
    const notify = useNotifySetting();
    const vibrate = useVibrateSetting();
    const reading = useReadingSettings();
    const selectedFont = getFontPreset(reading.fontVariant);
    const settings = useStoreSelector((snapshot) => snapshot.settings);
    const autoResume = settings[AUTO_RESUME_DEFAULT_KEY] === true;
    return (
        <Box
            sx={{
                display: "grid",
                gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
                gap: 2,
                alignItems: "start",
                "@media (max-width: 1279px)": {
                    gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
                },
            }}
        >
            <DesktopModalBlock label="Appearance">
                <DesktopSettingsRow shortcut="T" shortcutAvailable={shortcutsAvailable} label="Theme" description="Follow the system or pin a palette">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 0.75 }}>
                        {(["system", "light", "dark"] as const).map((mode) => (
                            <DesktopSettingsChoice key={mode} active={themeMode === mode} onClick={() => onSetThemeMode(mode)} ariaLabel={`${mode} theme`}>
                                {mode.charAt(0).toUpperCase() + mode.slice(1)}
                            </DesktopSettingsChoice>
                        ))}
                    </Box>
                </DesktopSettingsRow>
                <DesktopSettingsRow shortcut="F" shortcutAvailable={shortcutsAvailable} label="Reading font" description="Typeface used for transcript prose">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 0.75 }}>
                        {FONT_PRESETS.map((preset) => (
                            <DesktopSettingsChoice key={preset.id} active={reading.fontVariant === preset.id} onClick={() => setFontVariant(preset.id)} ariaLabel={`${preset.label} reading font`}>
                                <Typography component="span" variant="body2" sx={{ fontFamily: preset.stack }} noWrap>{preset.label} · Aa 阅读</Typography>
                            </DesktopSettingsChoice>
                        ))}
                    </Box>
                </DesktopSettingsRow>
                <Box sx={{ mx: 1.5, mt: 1, p: 1.5, borderRadius: 1.5, bgcolor: "action.hover", border: 1, borderColor: "divider" }}>
                    <Typography variant="overline" color="text.secondary">Live preview</Typography>
                    <Typography
                        sx={{
                            mt: 0.5,
                            px: `${reading.padding}px`,
                            fontFamily: selectedFont.stack,
                            // Global font scale is already applied at <html>.
                            // Keep the preview at one inherited rem so changing
                            // Font size is represented once, not squared here.
                            fontSize: "1rem",
                            lineHeight: reading.lineHeight,
                        }}
                    >
                        Make the common path fast. 阅读输出时，密度、节奏和字形会立即反映在这里。
                    </Typography>
                </Box>
            </DesktopModalBlock>
            <DesktopModalBlock label="Density">
                <DesktopSettingsRow shortcut="Z" shortcutAvailable={shortcutsAvailable} label="Font size" description="Scale application text">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(4, minmax(0, 1fr))", gap: 0.75 }}>
                        {FONT_SCALE_PRESETS.map((value) => <DesktopSettingsChoice key={value} active={nearestPreset(reading.fontScale, FONT_SCALE_PRESETS) === value} onClick={() => setFontScale(value)} ariaLabel={`${Math.round(value * 100)} percent font size`}>{Math.round(value * 100)}%</DesktopSettingsChoice>)}
                    </Box>
                </DesktopSettingsRow>
                <DesktopSettingsRow shortcut="P" shortcutAvailable={shortcutsAvailable} label="Padding" description="Transcript and composer side gutter">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(4, minmax(0, 1fr))", gap: 0.75 }}>
                        {PADDING_PRESETS.map((value) => <DesktopSettingsChoice key={value} active={nearestPreset(reading.padding, PADDING_PRESETS) === value} onClick={() => setPadding(value)} ariaLabel={`${value} pixel padding`}>{value}px</DesktopSettingsChoice>)}
                    </Box>
                </DesktopSettingsRow>
                <DesktopSettingsRow shortcut="R" shortcutAvailable={shortcutsAvailable} label="Line height" description="Vertical rhythm for long output">
                    <Box sx={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 0.75 }}>
                        {LINE_HEIGHT_PRESETS.map((value) => <DesktopSettingsChoice key={value} active={nearestPreset(reading.lineHeight, LINE_HEIGHT_PRESETS) === value} onClick={() => setLineHeight(value)} ariaLabel={`${value.toFixed(1)} line height`}>{value.toFixed(1)}</DesktopSettingsChoice>)}
                    </Box>
                </DesktopSettingsRow>
            </DesktopModalBlock>
            <DesktopModalBlock
                label="Workflow"
                sx={{ "@media (max-width: 1279px)": { gridColumn: "1 / -1" } }}
            >
                <Box
                    sx={{
                        display: "grid",
                        gridTemplateColumns: "1fr",
                        columnGap: 2,
                        "@media (max-width: 1279px)": {
                            gridTemplateColumns: "1fr 1fr",
                        },
                    }}
                >
                    <DesktopSettingsRow shortcut="S" shortcutAvailable={shortcutsAvailable} label="Sound alert" description="Chime when an agent needs you">
                        <DesktopSettingsChoice active={notify} onClick={() => setNotifySetting(!notify)} ariaLabel="Toggle sound alerts">{notify ? "On" : "Off"}</DesktopSettingsChoice>
                    </DesktopSettingsRow>
                    <DesktopSettingsRow shortcut="V" shortcutAvailable={shortcutsAvailable} label="Vibration alert" description="Native haptic notification">
                        <DesktopSettingsChoice active={vibrate} onClick={() => setVibrateSetting(!vibrate)} ariaLabel="Toggle vibration alerts">{vibrate ? "On" : "Off"}</DesktopSettingsChoice>
                    </DesktopSettingsRow>
                    <DesktopSettingsRow shortcut="M" shortcutAvailable={shortcutsAvailable} label="Vim keybindings" description="Modal editing in the composer">
                        <DesktopSettingsChoice active={vim} onClick={() => setVimSetting(!vim)} ariaLabel="Toggle Vim keybindings">{vim ? "On" : "Off"}</DesktopSettingsChoice>
                    </DesktopSettingsRow>
                    <DesktopSettingsRow shortcut="A" shortcutAvailable={shortcutsAvailable} label="Auto-resume" description="Continue interrupted turns after restart">
                        <DesktopSettingsChoice active={autoResume} onClick={() => setSetting(AUTO_RESUME_DEFAULT_KEY, !autoResume)} ariaLabel="Toggle automatic turn resume">{autoResume ? "On" : "Off"}</DesktopSettingsChoice>
                    </DesktopSettingsRow>
                </Box>
                <Divider sx={{ my: 1 }} />
                <AutoResumeSettings showToggle={false} />
            </DesktopModalBlock>
        </Box>
    );
}

// Global auto-resume settings (tasks/archive/2026/07/session-auto-resume): the default
// toggle + a collapsed continuation-template editor with a live interpolated
// preview. Server-authoritative (reads `state.settings`, writes via setSetting).
function AutoResumeSettings({ showToggle = true }: { showToggle?: boolean } = {}): React.JSX.Element {
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
            {showToggle && <Typography variant="overline" color="text.secondary">
                Interrupted turns
            </Typography>}
            {showToggle && <Stack
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
            </Stack>}
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

type MachineEventView =
    | { event: "login_challenge"; request_id: string; provider: string; verification_url: string; user_code?: string; input_required?: boolean; expires_at_ms: number }
    | { event: "login_state"; request_id: string; provider: string; state: string; account_label?: string; detail?: string }
    | { event: "command_result"; request_id: string; accepted: boolean; detail?: string }
    | { event: "inventory"; observed_at_ms: number; components: unknown[] };

function machineProviderName(slot?: string): string {
    if (slot === "codex") return "Codex";
    if (slot === "claude" || slot === "claude-code") return "Claude Code";
    if (slot === "gemini") return "Gemini";
    return slot || "Agent";
}

function machineComponentName(component: MachineChoice["components"][number]): string {
    if (component.id.kind === "provider_cli") return machineProviderName(component.id.slot);
    if (component.id.kind === "provider_adapter") return `${machineProviderName(component.id.slot)} adapter`;
    if (component.id.kind === "zed_server") return "Zed";
    if (component.id.kind === "zed_adapter") return "Zed adapter";
    if (component.id.kind === "code_adapter") return "Code adapter";
    if (component.id.kind === "machine_host") return "Machine host";
    if (component.id.kind === "acp_runtime") return "ACP runtime";
    if (component.id.kind === "managed_node") return "Managed Node";
    return component.id.slot || component.id.kind.replaceAll("_", " ");
}

function MachinesContent(): React.JSX.Element {
    const [machines, setMachines] = useState<readonly MachineChoice[]>([]);
    const [events, setEvents] = useState<Record<string, readonly MachineEventView[]>>({});
    const [expanded, setExpanded] = useState<Record<string, boolean>>({});
    const [busy, setBusy] = useState<Record<string, boolean>>({});
    const [loginCodes, setLoginCodes] = useState<Record<string, string>>({});
    const [componentErrors, setComponentErrors] = useState<Record<string, string>>({});
    const [updateConfirmation, setUpdateConfirmation] = useState<{
        machineId: string;
        components: readonly MachineChoice["components"][number][];
        action: "npm" | "reconcile-one" | "reconcile-all";
    } | null>(null);
    const loadEvents = useCallback((machineId: string): void => {
        void fetch(`/api/machines/${encodeURIComponent(machineId)}/events`)
            .then((response) => response.ok ? response.json() : [])
            .then((value: MachineEventView[]) => {
                setEvents((current) => ({ ...current, [machineId]: Array.isArray(value) ? value : [] }));
            });
    }, []);
    const refresh = useCallback((): void => {
        void fetch("/api/machines")
            .then((response) => response.ok ? response.json() : [])
            .then((value: MachineChoice[]) => {
                const next = Array.isArray(value) ? value : [];
                setMachines(next);
                next.forEach((machine) => loadEvents(machine.id));
            });
    }, [loadEvents]);
    useEffect(() => {
        refresh();
        const timer = globalThis.setInterval(refresh, 2_000);
        return () => globalThis.clearInterval(timer);
    }, [refresh]);
    const command = (machineId: string, action: "refresh" | "login" | "components/reconcile", provider?: string): void => {
        const busyKey = `${machineId}:${action}:${provider ?? ""}`;
        setBusy((current) => ({ ...current, [busyKey]: true }));
        void fetch(`/api/machines/${encodeURIComponent(machineId)}/${action}`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            ...(action === "login" ? { body: JSON.stringify({ provider }) } : {}),
        }).finally(() => {
            setBusy((current) => ({ ...current, [busyKey]: false }));
            globalThis.setTimeout(() => {
                refresh();
                loadEvents(machineId);
            }, 500);
        });
    };
    const updateOne = (machineId: string, component: MachineChoice["components"][number]): void => {
        const key = `${machineId}:component:${component.id.kind}:${component.id.slot ?? ""}`;
        setBusy((current) => ({ ...current, [key]: true }));
        void fetch(`/api/machines/${encodeURIComponent(machineId)}/components/reconcile-one`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ kind: component.id.kind, slot: component.id.slot ?? "" }),
        }).finally(() => {
            setBusy((current) => ({ ...current, [key]: false }));
            globalThis.setTimeout(refresh, 500);
        });
    };
    const submitLoginCode = (machineId: string, requestId: string): void => {
        const code = loginCodes[requestId]?.trim() ?? "";
        if (!code) return;
        const key = `${machineId}:login-code:${requestId}`;
        setBusy((current) => ({ ...current, [key]: true }));
        void fetch(`/api/machines/${encodeURIComponent(machineId)}/login/${encodeURIComponent(requestId)}`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ code }),
        }).then((response) => {
            if (response.ok) setLoginCodes((current) => ({ ...current, [requestId]: "" }));
        }).finally(() => {
            setBusy((current) => ({ ...current, [key]: false }));
            globalThis.setTimeout(() => loadEvents(machineId), 300);
        });
    };
    const updateNpm = (machineId: string, component: MachineChoice["components"][number]): void => {
        const key = `${machineId}:npm:${component.id.kind}:${component.id.slot ?? ""}`;
        setBusy((current) => ({ ...current, [key]: true }));
        setComponentErrors((current) => ({ ...current, [key]: "" }));
        void (async () => {
            try {
                const response = await fetch(`/api/machines/${encodeURIComponent(machineId)}/components/update-npm`, {
                    method: "POST",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify({ kind: component.id.kind, slot: component.id.slot ?? "" }),
                });
                if (!response.ok) throw new Error(await response.text() || "Could not start update");
                const { request_id: requestId } = await response.json() as { request_id: string };
                for (let attempt = 0; attempt < 180; attempt += 1) {
                    await new Promise((resolve) => globalThis.setTimeout(resolve, 1_000));
                    const eventResponse = await fetch(`/api/machines/${encodeURIComponent(machineId)}/events`);
                    const machineEvents = eventResponse.ok ? await eventResponse.json() as MachineEventView[] : [];
                    const result = machineEvents.find((event) =>
                        event.event === "command_result" && event.request_id === requestId
                    );
                    if (result?.event === "command_result") {
                        if (!result.accepted) throw new Error(result.detail || "Update failed");
                        refresh();
                        loadEvents(machineId);
                        return;
                    }
                }
                throw new Error("Update is still running; refresh to check its result");
            } catch (error) {
                setComponentErrors((current) => ({
                    ...current,
                    [key]: error instanceof Error ? error.message : "Update failed",
                }));
            } finally {
                setBusy((current) => ({ ...current, [key]: false }));
            }
        })();
    };
    const requestNpmUpdate = (machineId: string, component: MachineChoice["components"][number]): void => {
        if (component.active_leases > 0) {
            setUpdateConfirmation({ machineId, components: [component], action: "npm" });
            return;
        }
        updateNpm(machineId, component);
    };
    const requestReconcileOne = (machineId: string, component: MachineChoice["components"][number]): void => {
        if (component.active_leases > 0) {
            setUpdateConfirmation({ machineId, components: [component], action: "reconcile-one" });
            return;
        }
        updateOne(machineId, component);
    };
    const requestReconcileAll = (machine: MachineChoice): void => {
        const affected = machine.components.filter((component) =>
            component.active_leases > 0 && (machine.pending_updates ?? []).some((id) =>
                id.kind === component.id.kind && (id.slot ?? "") === (component.id.slot ?? "")
            )
        );
        if (affected.length > 0) {
            setUpdateConfirmation({ machineId: machine.id, components: affected, action: "reconcile-all" });
            return;
        }
        command(machine.id, "components/reconcile");
    };
    const confirmMachineUpdate = (): void => {
        const pending = updateConfirmation;
        if (!pending) return;
        setUpdateConfirmation(null);
        const component = pending.components[0];
        if (pending.action === "npm" && component) updateNpm(pending.machineId, component);
        else if (pending.action === "reconcile-one" && component) updateOne(pending.machineId, component);
        else if (pending.action === "reconcile-all") command(pending.machineId, "components/reconcile");
    };
    useConfirmEnter(updateConfirmation !== null, confirmMachineUpdate);
    return (
        <Stack spacing={2}>
            <Box>
                <Typography fontWeight={760}>Machines</Typography>
                <Typography variant="caption" color="text.secondary">Where Cowboy sessions run</Typography>
            </Box>
            {machines.map((machine) => {
                const latest = events[machine.id]?.at(-1);
                const open = Boolean(expanded[machine.id]);
                const pending = machine.pending_updates ?? [];
                const projectWorkspaces = machine.workspaces.filter((workspace) =>
                    workspace.id !== "home" && workspace.id !== "columbus"
                );
                const rootWorkspaces = machine.workspaces.filter((workspace) =>
                    workspace.id === "home" || workspace.id === "columbus"
                );
                const visibleComponents = machine.components.filter((component) =>
                    component.state !== "missing" ||
                    component.id.kind === "provider_cli" ||
                    component.id.kind === "zed_server" ||
                    component.id.kind === "zed_adapter" ||
                    pending.some((id) =>
                        id.kind === component.id.kind && (id.slot ?? "") === (component.id.slot ?? "")
                    )
                );
                const providerComponents = machine.components.filter((component) =>
                    component.id.kind === "provider_cli"
                );
                const zedComponent = machine.components.find((component) =>
                    component.id.kind === "zed_server"
                );
                const componentSections = [
                    {
                        label: "Agents",
                        components: visibleComponents.filter((component) =>
                            component.id.kind === "provider_cli" || component.id.kind === "provider_adapter"
                        ),
                    },
                    {
                        label: "Integrations",
                        components: visibleComponents.filter((component) =>
                            component.id.kind === "zed_server" || component.id.kind === "zed_adapter" || component.id.kind === "code_adapter"
                        ),
                    },
                    {
                        label: "Runtime",
                        components: visibleComponents.filter((component) =>
                            component.id.kind !== "provider_cli" &&
                            component.id.kind !== "provider_adapter" &&
                            component.id.kind !== "zed_server" &&
                            component.id.kind !== "zed_adapter" &&
                            component.id.kind !== "code_adapter"
                        ),
                    },
                ].filter((section) => section.components.length > 0);
                const presence = machinePresencePresentation(machine.status);
                return (
                    <Paper
                        key={machine.id}
                        variant="outlined"
                        sx={{
                            borderRadius: 3,
                            overflow: "hidden",
                            borderColor: open ? "primary.main" : "divider",
                            transition: "border-color .2s",
                        }}
                    >
                        <Stack spacing={1.25} sx={{ p: 1.5 }}>
                            <Stack direction="row" alignItems="center" spacing={1.25}>
                                <StatusDot status={presence.indicator} />
                                <Box sx={{ minWidth: 0, flex: 1 }}>
                                    <Stack direction="row" spacing={0.75} alignItems="baseline" flexWrap="wrap" useFlexGap>
                                        <Typography fontWeight={740}>{machine.display_name}</Typography>
                                        <Typography variant="caption" color="text.secondary">
                                            {machine.platform} · {machine.architecture}{machine.local ? " · Local server" : ""}
                                        </Typography>
                                    </Stack>
                                    <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.1 }}>
                                        {machine.workspaces.length} workspace{machine.workspaces.length === 1 ? "" : "s"} · {machine.active_sessions} active session{machine.active_sessions === 1 ? "" : "s"}
                                        {` · ${machine.capacity.max_sessions} max`}
                                    </Typography>
                                </Box>
                                <Chip
                                    size="small"
                                    label={machine.schedulable
                                        ? "Ready"
                                        : machine.capacity.draining
                                        ? "Draining"
                                        : presence.label}
                                    color={machine.schedulable ? "success" : "default"}
                                    sx={{ fontWeight: 650 }}
                                />
                            </Stack>
                            <Stack spacing={0.75}>
                                    <Stack direction="row" spacing={1} alignItems="center">
                                        <Typography variant="overline" color="text.secondary" sx={{ width: 72, flexShrink: 0 }}>Projects</Typography>
                                        <Chip
                                            size="small"
                                            variant="outlined"
                                            label={`${projectWorkspaces.length} project${projectWorkspaces.length === 1 ? "" : "s"}`}
                                            title={projectWorkspaces.map((workspace) => workspace.display_name).join(", ")}
                                        />
                                    </Stack>
                                    <Stack direction="row" spacing={1} alignItems="flex-start">
                                        <Typography variant="overline" color="text.secondary" sx={{ width: 72, flexShrink: 0, pt: 0.35 }}>Agents</Typography>
                                        <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap sx={{ minWidth: 0 }}>
                                            {providerComponents.map((component) => {
                                                const unavailable = component.state === "missing";
                                                const failed = component.state === "failed" || component.auth === "error" || component.auth === "expired";
                                                const ready = component.state === "active" && component.auth === "signed_in";
                                                const updateAvailable = component.update?.available === true;
                                                const state = unavailable ? "unavailable" : failed ? "error" : component.auth === "signed_out" ? "sign in" : updateAvailable ? "update" : ready ? "ready" : component.state;
                                                return (
                                                    <Chip
                                                        key={component.id.slot}
                                                        size="small"
                                                        variant="outlined"
                                                        color={failed ? "error" : updateAvailable ? "warning" : ready ? "success" : "default"}
                                                        label={`${machineProviderName(component.id.slot)} · ${state}`}
                                                    />
                                                );
                                            })}
                                        </Stack>
                                    </Stack>
                                    <Stack direction="row" spacing={1} alignItems="center">
                                        <Typography variant="overline" color="text.secondary" sx={{ width: 72, flexShrink: 0 }}>Integrations</Typography>
                                        <Chip
                                            size="small"
                                            variant="outlined"
                                            color={zedComponent?.state === "active" ? "success" : zedComponent?.state === "failed" ? "error" : "default"}
                                            label={`Zed · ${zedComponent?.state === "active" ? "ready" : zedComponent?.state === "failed" ? "error" : "unavailable"}`}
                                        />
                                    </Stack>
                            </Stack>
                            {pending.length > 0 && (
                                <Button
                                    size="small"
                                    variant="contained"
                                    startIcon={busy[`${machine.id}:components/reconcile:`] ? <CircularProgress size={14} color="inherit" /> : <SystemUpdateAlt />}
                                    disabled={busy[`${machine.id}:components/reconcile:`]}
                                    onClick={() => requestReconcileAll(machine)}
                                    sx={{ alignSelf: "flex-start" }}
                                >
                                    Update all ({pending.length})
                                </Button>
                            )}
                            <ButtonBase
                                onClick={() => setExpanded((current) => ({ ...current, [machine.id]: !open }))}
                                sx={{
                                    alignSelf: "stretch",
                                    justifyContent: "space-between",
                                    minHeight: 40,
                                    px: 0.5,
                                    borderRadius: 1.5,
                                    color: "text.secondary",
                                }}
                            >
                                <Typography variant="body2" fontWeight={650}>
                                    {open ? "Hide details" : "Details & versions"}
                                </Typography>
                                {open ? <ExpandLess fontSize="small" /> : <ExpandMore fontSize="small" />}
                            </ButtonBase>
                            {open && (
                                <Stack spacing={1.25} sx={{ pt: 0.25 }}>
                                    <Stack spacing={0.75}>
                                        <Typography variant="overline" color="text.secondary">Projects</Typography>
                                        <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap>
                                            {projectWorkspaces.map((workspace) => (
                                                <Chip
                                                    key={workspace.id}
                                                    size="small"
                                                    variant="outlined"
                                                    label={workspace.display_name}
                                                    title={workspace.canonical_path}
                                                />
                                            ))}
                                        </Stack>
                                        {rootWorkspaces.length > 0 && (
                                            <Typography variant="caption" color="text.secondary">
                                                Roots: {rootWorkspaces.map((workspace) => workspace.display_name).join(" · ")}
                                            </Typography>
                                        )}
                                    </Stack>
                                    <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
                                            <Button
                                                size="small"
                                                startIcon={busy[`${machine.id}:refresh:`] ? <CircularProgress size={14} /> : <RefreshIcon />}
                                                disabled={busy[`${machine.id}:refresh:`]}
                                                onClick={() => command(machine.id, "refresh")}
                                            >Refresh</Button>
                                    </Stack>
                                    {componentSections.map((section) => (
                                        <Stack key={section.label} spacing={0.75}>
                                            <Typography variant="overline" color="text.secondary">{section.label}</Typography>
                                            {section.components.map((component) => {
                                                const componentPending = pending.some((id) =>
                                                    id.kind === component.id.kind && (id.slot ?? "") === (component.id.slot ?? "")
                                                );
                                                const componentKey = `${machine.id}:component:${component.id.kind}:${component.id.slot ?? ""}`;
                                                const update = component.update;
                                                const release = machineVersionPresentation(
                                                    component.version,
                                                    component.state,
                                                    update,
                                                    componentPending,
                                                );
                                                const updateTitle = update
                                                    ? `Checked ${new Date(update.checked_at_ms).toLocaleString()} via ${update.source}`
                                                    : "No authoritative release comparison is available";
                                                const provider = component.id.kind === "provider_cli" ? component.id.slot : undefined;
                                                const authEvent = provider
                                                    ? [...(events[machine.id] ?? [])].reverse().find((event) =>
                                                        (event.event === "login_challenge" || event.event === "login_state") && event.provider === provider
                                                    )
                                                    : undefined;
                                                const loginChallenge = authEvent?.event === "login_challenge" && authEvent.expires_at_ms > Date.now()
                                                    ? authEvent
                                                    : undefined;
                                                const loginPending = authEvent?.event === "login_challenge" ||
                                                    (authEvent?.event === "login_state" && authEvent.state === "pending");
                                                const loginBusyKey = `${machine.id}:login:${provider ?? ""}`;
                                                const loginBusy = busy[loginBusyKey] || loginPending;
                                                const npmUpdateKey = `${machine.id}:npm:${component.id.kind}:${component.id.slot ?? ""}`;
                                                const npmUpdating = busy[npmUpdateKey];
                                                const npmInstallable = update?.available === true && update.installable;
                                                return (
                                                    <Stack
                                                        key={`${component.id.kind}:${component.id.slot ?? ""}`}
                                                        direction="row"
                                                        spacing={1}
                                                        alignItems="center"
                                                        flexWrap="wrap"
                                                        useFlexGap
                                                        sx={{
                                                            minWidth: 0,
                                                            py: 0.75,
                                                            px: 1,
                                                            borderRadius: 1.5,
                                                            bgcolor: "action.hover",
                                                        }}
                                                    >
                                                        <Box sx={{ minWidth: 0, flex: 1 }}>
                                                            <Typography variant="body2" fontWeight={650}>
                                                                {machineComponentName(component)}
                                                            </Typography>
                                                            <Typography
                                                                variant="caption"
                                                                color="text.secondary"
                                                                title={updateTitle}
                                                                sx={{ display: "block", overflowWrap: "anywhere" }}
                                                            >
                                                                {release.version}
                                                                {component.auth ? ` · ${component.auth.replaceAll("_", " ")}` : ""}
                                                                {component.generation ? ` · generation ${component.generation}` : ""}
                                                            </Typography>
                                                        </Box>
                                                        {provider && component.auth === "signed_in" && (
                                                            <Chip size="small" color="success" variant="outlined" label="Signed in" />
                                                        )}
                                                        {provider && component.auth !== "signed_in" && (
                                                            <Button
                                                                size="small"
                                                                variant={loginBusy ? "outlined" : "contained"}
                                                                disabled={loginBusy}
                                                                startIcon={loginBusy ? <CircularProgress size={14} /> : undefined}
                                                                onClick={() => command(machine.id, "login", provider)}
                                                            >{loginBusy ? "Waiting" : "Sign in"}</Button>
                                                        )}
                                                        {provider && npmInstallable && (
                                                            <Button
                                                                size="small"
                                                                variant="outlined"
                                                                color="warning"
                                                                disabled={npmUpdating}
                                                                startIcon={npmUpdating ? <CircularProgress size={14} color="inherit" /> : <SystemUpdateAlt />}
                                                                onClick={() => requestNpmUpdate(machine.id, component)}
                                                            >{npmUpdating ? "Updating" : "Update"}</Button>
                                                        )}
                                                        {!provider && componentPending && (
                                                            <Button
                                                                size="small"
                                                                variant="outlined"
                                                                color="warning"
                                                                disabled={busy[componentKey]}
                                                                onClick={() => requestReconcileOne(machine.id, component)}
                                                            >{busy[componentKey] ? <CircularProgress size={14} /> : "Update"}</Button>
                                                        )}
                                                        {!provider && !componentPending && npmInstallable && (
                                                            <Button
                                                                size="small"
                                                                variant="outlined"
                                                                color="warning"
                                                                disabled={npmUpdating}
                                                                startIcon={npmUpdating ? <CircularProgress size={14} color="inherit" /> : <SystemUpdateAlt />}
                                                                onClick={() => requestNpmUpdate(machine.id, component)}
                                                            >{npmUpdating ? "Updating" : "Update"}</Button>
                                                        )}
                                                        {!provider && !componentPending && !npmInstallable && (
                                                            <Chip
                                                                size="small"
                                                                variant="outlined"
                                                                color={release.tone}
                                                                label={release.status}
                                                                title={updateTitle}
                                                            />
                                                        )}
                                                        {componentErrors[npmUpdateKey] && (
                                                            <Alert severity="error" sx={{ width: "100%" }}>
                                                                {componentErrors[npmUpdateKey]}
                                                            </Alert>
                                                        )}
                                                        {loginChallenge && (
                                                            <Alert
                                                                severity="info"
                                                                sx={{ width: "100%", alignItems: "flex-start", mt: 0.25 }}
                                                            >
                                                                <Stack spacing={1} sx={{ minWidth: 0 }}>
                                                                    <Typography variant="body2" fontWeight={650}>
                                                                        Finish signing in to {machineProviderName(provider)}
                                                                    </Typography>
                                                                    <Typography variant="caption" color="text.secondary">
                                                                        {loginChallenge.input_required
                                                                            ? "Open the sign-in page, approve access, then paste the authorization code shown by the browser."
                                                                            : "Open the sign-in page and enter the device code. Cowboy will finish automatically."}
                                                                    </Typography>
                                                                    <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap>
                                                                        <Button
                                                                            size="small"
                                                                            variant="contained"
                                                                            component="a"
                                                                            href={loginChallenge.verification_url}
                                                                            target="_blank"
                                                                            rel="noreferrer"
                                                                        >Open sign-in page</Button>
                                                                        {loginChallenge.user_code && (
                                                                            <Button
                                                                                size="small"
                                                                                variant="outlined"
                                                                                onClick={() => void navigator.clipboard.writeText(loginChallenge.user_code ?? "")}
                                                                            >Copy {loginChallenge.user_code}</Button>
                                                                        )}
                                                                        <Button
                                                                            size="small"
                                                                            color="inherit"
                                                                            onClick={() => {
                                                                                void fetch(
                                                                                    `/api/machines/${encodeURIComponent(machine.id)}/login/${encodeURIComponent(loginChallenge.request_id)}`,
                                                                                    { method: "DELETE" },
                                                                                ).finally(() => loadEvents(machine.id));
                                                                            }}
                                                                        >Cancel</Button>
                                                                    </Stack>
                                                                    {loginChallenge.input_required && (
                                                                        <Stack direction={{ xs: "column", sm: "row" }} spacing={0.75}>
                                                                            <TextField
                                                                                size="small"
                                                                                fullWidth
                                                                                label="Authorization code"
                                                                                value={loginCodes[loginChallenge.request_id] ?? ""}
                                                                                autoComplete="off"
                                                                                onChange={(event) => setLoginCodes((current) => ({
                                                                                    ...current,
                                                                                    [loginChallenge.request_id]: event.target.value,
                                                                                }))}
                                                                                onKeyDown={(event) => {
                                                                                    if (event.key === "Enter") submitLoginCode(machine.id, loginChallenge.request_id);
                                                                                }}
                                                                            />
                                                                            <Button
                                                                                variant="contained"
                                                                                disabled={!loginCodes[loginChallenge.request_id]?.trim() || busy[`${machine.id}:login-code:${loginChallenge.request_id}`]}
                                                                                onClick={() => submitLoginCode(machine.id, loginChallenge.request_id)}
                                                                            >{busy[`${machine.id}:login-code:${loginChallenge.request_id}`] ? <CircularProgress size={16} /> : "Continue"}</Button>
                                                                        </Stack>
                                                                    )}
                                                                </Stack>
                                                            </Alert>
                                                        )}
                                                    </Stack>
                                                );
                                            })}
                                        </Stack>
                                    ))}
                                </Stack>
                            )}
                            {latest && latest.event !== "inventory" && (
                                <Typography variant="caption" color="text.secondary">
                                    {latest.event === "command_result" ? latest.detail ?? (latest.accepted ? "Command accepted" : "Command rejected") :
                                        latest.event === "login_state" ? `${latest.provider}: ${latest.state}${latest.detail ? ` · ${latest.detail}` : ""}` : ""}
                                </Typography>
                            )}
                        </Stack>
                    </Paper>
                );
            })}
            <Dialog
                open={updateConfirmation !== null}
                onClose={() => setUpdateConfirmation(null)}
                fullWidth
                maxWidth="xs"
            >
                <DialogTitle>Roll out this update?</DialogTitle>
                <DialogContent>
                    {updateConfirmation && (
                        <Stack spacing={2} sx={{ pt: 0.5 }}>
                            <Typography variant="body2">
                                {updateConfirmation.components.map(machineComponentName).join(", ")} {updateConfirmation.components.length === 1 ? "is" : "are"} used by active sessions on this Machine.
                            </Typography>
                            <Stack spacing={0.5}>
                                {updateConfirmation.components.map((component) => (
                                    <Typography key={`${component.id.kind}:${component.id.slot ?? ""}`} variant="caption" color="text.secondary">
                                        {machineComponentName(component)} · {component.active_leases} active {component.active_leases === 1 ? "session" : "sessions"}
                                    </Typography>
                                ))}
                            </Stack>
                            <Alert severity="warning">
                                Current turns will finish first. Cowboy will then replace affected workers gradually. Each session may be briefly unavailable while it reconnects; its transcript and agent session are preserved. Machine host or runtime updates may also reconnect the Machine service.
                            </Alert>
                            <Stack direction="row" spacing={1} justifyContent="flex-end">
                                <Button color="inherit" onClick={() => setUpdateConfirmation(null)}>
                                    Cancel
                                    <Kbd keys="Esc" />
                                </Button>
                                <Button
                                    variant="contained"
                                    color="warning"
                                    onClick={confirmMachineUpdate}
                                >
                                    Update and roll out
                                    <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
                                </Button>
                            </Stack>
                        </Stack>
                    )}
                </DialogContent>
            </Dialog>
        </Stack>
    );
}

function closestScrollableSettingsSurface(panel: HTMLElement): HTMLElement {
    let candidate: HTMLElement | null = panel;
    while (candidate) {
        const style = getComputedStyle(candidate);
        if (
            /(auto|scroll)/.test(style.overflowY) &&
            candidate.scrollHeight > candidate.clientHeight
        ) return candidate;
        candidate = candidate.parentElement;
    }
    return panel;
}

function isSettingsEditableTarget(target: EventTarget | null): target is HTMLElement {
    return target instanceof HTMLElement && (
        target.matches("input, textarea, select, [contenteditable='true']") ||
        target.closest("[contenteditable='true']") !== null
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
    const [tab, setTab] = useState<"settings" | "machines" | "info" | "logs">(initialTab);
    useEffect(() => {
        if (open) setTab(initialTab);
    }, [open, initialTab]);
    const vim = useVimSetting();
    const notify = useNotifySetting();
    const vibrate = useVibrateSetting();
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
    // Surface policy for this content-heavy modal:
    // - phones (<768px) stay a bottom sheet even with a fine pointer;
    // - portrait/mid-size touch tablets (<1024px, coarse pointer) keep the
    //   touch-native sheet;
    // - landscape tablets and desktop windows (>=1024px), plus pointer-driven
    //   devices from 768px up, use the centered dialog.
    const useSheetSurface = useMediaQuery(
        "(max-width: 767.95px), (min-width: 768px) and (max-width: 1023.95px) and (pointer: coarse)",
    );
    // Vim is desktop-only (ComposerEditor won't load it on touch), so the
    // toggle only appears where a physical keyboard exists.
    const desktop = useMediaQuery("(pointer: fine) and (hover: hover)");
    const settingsPanelRef = useRef<HTMLDivElement>(null);
    const [settingsGoPrefix, setSettingsGoPrefix] = useState(false);
    const [settingsEditableFocus, setSettingsEditableFocus] = useState(false);
    useEffect(() => {
        if (!open || !desktop) return undefined;
        let goPending = false;
        let goTimer: number | null = null;
        const setGoPending = (pending: boolean): void => {
            goPending = pending;
            setSettingsGoPrefix(pending);
            if (goTimer !== null) globalThis.clearTimeout(goTimer);
            goTimer = pending
                ? globalThis.setTimeout(() => {
                    goPending = false;
                    setSettingsGoPrefix(false);
                }, 1200)
                : null;
        };
        const focusFirst = requestAnimationFrame(() => {
            settingsPanelRef.current?.querySelector<HTMLElement>(
                "[data-settings-choice]",
            )?.focus();
        });
        const onKeyDown = (event: KeyboardEvent): void => {
            if (isImeKeyEvent(event)) return;
            const target = event.target;
            if (isSettingsEditableTarget(target)) {
                if (event.key === "Escape") {
                    event.preventDefault();
                    event.stopPropagation();
                    target.blur();
                    const returnTarget = target.closest<HTMLElement>(
                        "[data-settings-row], [data-settings-machines], [data-settings-info], [data-settings-logs]",
                    );
                    requestAnimationFrame(() =>
                        (returnTarget ?? settingsPanelRef.current)?.focus({ preventScroll: true }));
                }
                return;
            }
            const key = workspaceCommandKey(event).toLowerCase();
            const tabs = ["settings", "machines", "info", "logs"] as const;
            const panel = settingsPanelRef.current;
            if (!panel) return;
            const scrollSurface = closestScrollableSettingsSurface(panel);
            if (event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && ["d", "u", "f", "b"].includes(key)) {
                event.preventDefault();
                event.stopPropagation();
                setGoPending(false);
                const direction = key === "d" || key === "f" ? 1 : -1;
                const distance = scrollSurface.clientHeight * (key === "d" || key === "u" ? 0.5 : 0.92);
                scrollSurface.scrollBy({ top: direction * distance, behavior: "auto" });
                return;
            }
            if (event.metaKey || event.ctrlKey || event.altKey) {
                setGoPending(false);
                return;
            }
            if (event.repeat && key === "g") {
                event.preventDefault();
                event.stopPropagation();
                return;
            }
            if (goPending) {
                event.preventDefault();
                event.stopPropagation();
                const goTop = !event.shiftKey && key === "g";
                setGoPending(false);
                if (goTop) scrollSurface.scrollTo({ top: 0, behavior: "auto" });
                return;
            }
            if (event.shiftKey && key === "g") {
                event.preventDefault();
                event.stopPropagation();
                setGoPending(false);
                scrollSurface.scrollTo({ top: scrollSurface.scrollHeight, behavior: "auto" });
                return;
            }
            if (!event.shiftKey && key === "g") {
                event.preventDefault();
                event.stopPropagation();
                setGoPending(true);
                return;
            }
            if (!desktop && (key === "[" || key === "]")) {
                event.preventDefault();
                const index = tabs.indexOf(tab);
                setTab(tabs[(index + (key === "]" ? 1 : tabs.length - 1)) % tabs.length] ?? "settings");
                return;
            }
            if (desktop && ["n", "i", "o"].includes(key)) {
                const section = panel.querySelector<HTMLElement>(
                    key === "n"
                        ? "[data-settings-machines]"
                        : key === "i"
                        ? "[data-settings-info]"
                        : "[data-settings-logs]",
                );
                if (section) {
                    event.preventDefault();
                    section.scrollIntoView({ block: "start", behavior: "auto" });
                    section.focus({ preventScroll: true });
                }
                return;
            }
            if (!desktop && tab !== "settings") return;
            const rows = [...panel.querySelectorAll<HTMLElement>("[data-settings-row]")];
            const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
            const activeRow = active?.closest<HTMLElement>("[data-settings-row]");
            const focusChoice = (row: HTMLElement, preferred = 0): void => {
                const choices = [...row.querySelectorAll<HTMLElement>("[data-settings-choice]")];
                const selected = choices.findIndex((choice) => choice.getAttribute("aria-pressed") === "true");
                choices[Math.max(0, Math.min(choices.length - 1, preferred < 0 ? selected : preferred))]?.focus();
            };
            const direct = panel.querySelector<HTMLElement>(`[data-settings-shortcut="${CSS.escape(key)}"]`);
            if (direct && !["h", "j", "k", "l"].includes(key)) {
                event.preventDefault();
                focusChoice(direct, -1);
                return;
            }
            if (!["h", "j", "k", "l"].includes(key)) return;
            event.preventDefault();
            if (desktop && !activeRow) {
                const controls = [...panel.querySelectorAll<HTMLElement>(
                    'button:not(:disabled), [href], input:not(:disabled), textarea:not(:disabled), select:not(:disabled), [tabindex="0"]',
                )].filter((control) => control.offsetParent !== null);
                const controlIndex = active ? controls.indexOf(active) : -1;
                const delta = key === "j" || key === "l" ? 1 : -1;
                controls[Math.max(0, Math.min(controls.length - 1, controlIndex + delta))]?.focus();
                return;
            }
            const rowIndex = activeRow ? rows.indexOf(activeRow) : -1;
            if (key === "j" || key === "k") {
                const next = rowIndex < 0
                    ? (key === "j" ? rows[0] : rows.at(-1))
                    : rows[Math.max(0, Math.min(rows.length - 1, rowIndex + (key === "j" ? 1 : -1)))];
                if (next) focusChoice(next, -1);
                return;
            }
            const row = activeRow ?? rows[0];
            if (!row) return;
            const choices = [...row.querySelectorAll<HTMLElement>("[data-settings-choice]")];
            const choiceIndex = active ? choices.indexOf(active) : -1;
            const next = choices[Math.max(0, Math.min(choices.length - 1, choiceIndex + (key === "l" ? 1 : -1)))];
            next?.focus();
        };
        const onFocusIn = (event: FocusEvent): void => {
            const panel = settingsPanelRef.current;
            if (
                panel?.contains(event.target as Node) &&
                isSettingsEditableTarget(event.target)
            ) setGoPending(false);
        };
        globalThis.addEventListener("keydown", onKeyDown, true);
        globalThis.addEventListener("focusin", onFocusIn, true);
        return () => {
            cancelAnimationFrame(focusFirst);
            if (goTimer !== null) globalThis.clearTimeout(goTimer);
            globalThis.removeEventListener("keydown", onKeyDown, true);
            globalThis.removeEventListener("focusin", onFocusIn, true);
            setSettingsGoPrefix(false);
            setSettingsEditableFocus(false);
        };
    }, [desktop, open, tab]);
    const settingsShortcutsAvailable = !settingsGoPrefix && !settingsEditableFocus;
    return (
        <Sheet
            open={open}
            onClose={onClose}
            forceSheet={useSheetSurface}
            wide
            cover
            desktopMaxWidth={1440}
        >
            {/* Mobile keeps progressive-disclosure tabs. Desktop is one visible
                keyboard workbench, so no information is hidden behind a tab. */}
            <Box
                sx={{
                    position: desktop ? "sticky" : "static",
                    top: desktop ? -1 : "auto",
                    zIndex: desktop ? 4 : "auto",
                    bgcolor: desktop ? "background.paper" : "transparent",
                }}
            >
            <Box
                sx={{
                    display: desktop ? "flex" : "grid",
                    gridTemplateColumns: desktop ? "none" : "1fr",
                    alignItems: "center",
                    mt: 0.25,
                    mb: desktop ? 1 : 1.5,
                    py: desktop ? 0.5 : 0,
                    width: "100%",
                }}
            >
                {desktop ? (
                    <Box sx={{ flex: 1 }}>
                        <Typography variant="h6" fontWeight={780}>Cowboy control center</Typography>
                        <Typography variant="caption" color="text.secondary">Preferences, runtime information, and automation history</Typography>
                    </Box>
                ) : null}
                {!desktop && <SegmentedPill
                    value={tab}
                    onChange={setTab}
                    options={[{ value: "settings", label: "Settings" }, { value: "machines", label: "Machines" }, { value: "info", label: "Info" }, { value: "logs", label: "Logs" }]}
                    sx={{
                        width: "100%",
                        justifySelf: "center",
                        "& .MuiButtonBase-root": {
                            flex: 1,
                            minWidth: 0,
                            px: 0.75,
                        },
                    }}
                />}
                {desktop && <Box
                    sx={{
                        flex: 1,
                        display: "flex",
                        justifyContent: "flex-end",
                        visibility: useSheetSurface ? "hidden" : "visible",
                        pointerEvents: useSheetSurface ? "none" : "auto",
                    }}
                >
                    <Box sx={{ position: "relative", display: "inline-flex" }}>
                        <IconButton aria-label="close settings" onClick={onClose} sx={{ width: 36, height: 36 }}>
                            <CloseIcon fontSize="small" />
                        </IconButton>
                        <Kbd
                            keys="Esc"
                            floating
                            availability={settingsShortcutsAvailable ? "available" : "inactive"}
                        />
                    </Box>
                </Box>}
            </Box>
            </Box>
            {desktop ? (
                <Box
                    ref={settingsPanelRef}
                    data-desktop-settings-workbench
                    tabIndex={-1}
                    onFocusCapture={(event): void => {
                        setSettingsEditableFocus(isSettingsEditableTarget(event.target));
                    }}
                    onBlurCapture={(): void => {
                        requestAnimationFrame(() => {
                            const panel = settingsPanelRef.current;
                            setSettingsEditableFocus(
                                panel !== null && panel.contains(document.activeElement) &&
                                isSettingsEditableTarget(document.activeElement),
                            );
                        });
                    }}
                    sx={{
                        flex: 1,
                        minHeight: 0,
                        overflowY: "auto",
                        display: "grid",
                        gridTemplateColumns: "minmax(0, 1fr)",
                        gap: 2,
                        alignItems: "start",
                        pr: 0.75,
                        pb: 2,
                        "@media (max-width: 1279px)": { gridTemplateColumns: "1fr" },
                    }}
                >
                    <DesktopSettingsContent
                        themeMode={themeMode}
                        onSetThemeMode={onSetThemeMode}
                        shortcutsAvailable={settingsShortcutsAvailable}
                    />
                    <DesktopModalBlock
                        label="Remote runtimes & credentials"
                        title="Machines"
                        shortcut="N"
                        shortcutAvailable={settingsShortcutsAvailable}
                        section="machines"
                    >
                        <MachinesContent />
                    </DesktopModalBlock>
                    <DesktopModalBlock
                        label="Runtime, usage & audit"
                        title="Info"
                        shortcut="I"
                        shortcutAvailable={settingsShortcutsAvailable}
                        section="info"
                    >
                        <InfoContent
                            desktop
                            aside={
                                <DesktopModalBlock
                                    label="Audit trail"
                                    title="Logs"
                                    shortcut="O"
                                    shortcutAvailable={settingsShortcutsAvailable}
                                    section="logs"
                                >
                                    <UsageLogs dense />
                                </DesktopModalBlock>
                            }
                        />
                    </DesktopModalBlock>
                </Box>
            ) : tab === "machines" ? <MachinesContent /> : tab === "info" ? <InfoContent /> : tab === "logs" ? <UsageLogs /> : (
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
                    </>
                )}
                <Divider />
                <AutoResumeSettings />
                {/* Daemon system info (Storage metrics + About) lives in the Info
                    tab — Settings holds user preferences only. */}
            </Stack>
            )}
            {desktop && (
                <Box sx={{ flex: "0 0 auto", mx: -3 }}>
                    <DesktopShortcutBar
                            groups={[
                                {
                                    label: "Navigate",
                                    slots: [
                                        { shortcut: "J/K", label: "Field", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        { shortcut: "H/L", label: "Choice", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        { shortcut: ENTER_LABEL, label: "Apply", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                    ],
                                },
                                {
                                    label: "Page",
                                    slots: [
                                        { shortcut: "Ctrl+D/U", label: "Half", availability: settingsEditableFocus ? "inactive" : "available" },
                                        { shortcut: "Ctrl+F/B", label: "Full", availability: settingsEditableFocus ? "inactive" : "available" },
                                        {
                                            shortcut: "G",
                                            label: "Go",
                                            availability: sequentialShortcutAvailability({
                                                scopeAvailable: !settingsEditableFocus,
                                                armed: settingsGoPrefix,
                                                prefix: true,
                                            }),
                                        },
                                        {
                                            shortcut: "G",
                                            label: "Top",
                                            availability: sequentialShortcutAvailability({
                                                scopeAvailable: !settingsEditableFocus,
                                                armed: settingsGoPrefix,
                                                prefix: false,
                                            }),
                                        },
                                        { shortcut: "Shift+G", label: "Bottom", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                    ],
                                },
                                {
                                    label: "Jump",
                                    slots: ([
                                            ["T", "Theme"], ["F", "Reading font"],
                                            ["Z", "Font size"], ["P", "Padding"],
                                            ["R", "Line height"], ["S", "Sound"],
                                            ["V", "Vibration"], ["M", "Vim"],
                                            ["A", "Auto-resume"],
                                        ] as const).map(([shortcut, title]) => ({
                                            shortcut,
                                            title,
                                            availability: settingsShortcutsAvailable ? "available" as const : "inactive" as const,
                                        })),
                                },
                                {
                                    label: "Sections",
                                    slots: [
                                        { shortcut: "N", label: "Machines", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        { shortcut: "I", label: "Info", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        { shortcut: "O", label: "Logs", availability: settingsShortcutsAvailable ? "available" : "inactive" },
                                        {
                                            shortcut: "Esc",
                                            label: settingsGoPrefix
                                                ? "Cancel prefix"
                                                : settingsEditableFocus
                                                ? "Leave field"
                                                : "Close",
                                        },
                                    ],
                                },
                            ]}
                    />
                </Box>
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
    return (
        <Sheet
            forceSheet={navbarAtBottom}
            open
            onClose={onClose}
            title="Delete this session?"
            mobileDismiss="none"
            actions={
                <>
                    <Button onClick={onClose} color="inherit">
                        Cancel
                        <Kbd keys="Esc" />
                    </Button>
                    <Button onClick={onConfirm} color="error" variant="contained">
                        Delete
                        <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
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
    const desktop = useSurfaceProfile().kind === "desktop";
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
    const trimmed = value.trim();
    const canSave = session !== null && trimmed.length > 0 && trimmed !== session.title;
    const submit = (): void => {
        if (canSave) onConfirm(trimmed);
    };
    useConfirmEnter(session !== null && desktop, submit, { suppressBareEnter: false });
    if (!session) return null;
    return (
        <Sheet
            forceSheet={navbarAtBottom}
            open
            onClose={onClose}
            title="Rename session"
            mobileDismiss="none"
            actions={
                <>
                    <Button onClick={onClose} color="inherit">
                        Cancel
                        <Kbd keys="Esc" />
                    </Button>
                    <Button
                        onClick={submit}
                        onKeyDown={(e): void => {
                            if (
                                desktop && e.key === "Enter" &&
                                !e.metaKey && !e.ctrlKey &&
                                !isImeKeyEvent(e.nativeEvent)
                            ) e.preventDefault();
                        }}
                        variant="contained"
                        disabled={!canSave}
                    >
                        Save
                        <Kbd
                            keys={`${MOD_LABEL}${ENTER_LABEL}`}
                            availability={canSave ? "available" : "inactive"}
                        />
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
                    if (
                        e.key === "Enter" && !e.shiftKey &&
                        !isImeKeyEvent(e.nativeEvent)
                    ) {
                        e.preventDefault();
                        if (!desktop) submit();
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

function DesktopSessionInfoPanel(
    { session }: { session: SessionMeta | null },
): React.JSX.Element {
    const [info, setInfo] = useState<SessionInfoData | null>(null);
    const [error, setError] = useState(false);
    useEffect(() => {
        if (!session) {
            setInfo(null);
            return undefined;
        }
        setInfo(null);
        setError(false);
        const ctrl = new AbortController();
        void fetch(`/api/sessions/${encodeURIComponent(session.id)}/info`, {
            signal: ctrl.signal,
        })
            .then((response) =>
                response.ok
                    ? response.json() as Promise<SessionInfoData>
                    : Promise.reject(new Error("not found"))
            )
            .then(setInfo)
            .catch(() => {
                if (!ctrl.signal.aborted) setError(true);
            });
        return () => ctrl.abort();
    }, [session?.id]);

    return (
        <Box
            aria-label="Session information"
            sx={{
                minWidth: 0,
                p: 2,
                border: 1,
                borderColor: "divider",
                borderRadius: 2,
                bgcolor: (theme) => alpha(theme.palette.text.primary, 0.025),
            }}
        >
            <Typography variant="overline" color="text.secondary">Overview</Typography>
            {error
                ? <Typography color="error" variant="body2" sx={{ mt: 1 }}>Couldn't load session info.</Typography>
                : !info
                ? <Typography variant="body2" color="text.secondary" sx={{ mt: 1 }}>Loading…</Typography>
                : (
                    <Stack spacing={1.15} sx={{ mt: 1 }}>
                        <InfoRow k="Title" v={info.title} />
                        <InfoRow k="Status" v={info.status} />
                        <InfoRow k="Provider" v={info.provider} />
                        <InfoRow k="Directory" v={info.cwd} />
                        <Divider />
                        <InfoRow k="Events" v={info.event_count.toLocaleString()} />
                        <InfoRow k="Queued" v={String(info.queue_count)} />
                        <InfoRow k="Drafts" v={String(info.drafts_count)} />
                        <Divider />
                        <InfoRow k="Session ID" v={info.id} />
                    </Stack>
                )}
        </Box>
    );
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
            cover={navbarAtBottom}
            open
            onClose={onClose}
            title="Session info"
        >
            {body}
        </Sheet>
    );
}
