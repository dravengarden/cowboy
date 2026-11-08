import { desktopSize, glyphSize } from "./surface/desktopSize";
import { ChecklistOutlined } from "@mui/icons-material";
import { defaultDraftTitle } from "./documents/defaultDraftTitle";
import { CreateVariantPicker, DraftCreationDirectory, type CreateVariant } from "./CreateVariantPicker";
import { draftRepository } from "./documents/store";
import { openDrafts } from "./documents/navigation";
import { ReliableListItemButton } from "./ReliableListItemButton";
import { type DraftMetadata } from "./documents/model";
import { documentNotice } from "./documents/DocumentNotifications";
import { hibernateAvailability, hibernateSession } from "./sessionHibernate";
import { WorkspaceDraftRow, type WorkspaceDraftAction } from "./documents/WorkspaceDraft";
import { SessionTrashSection, TrashItemSheet, TrashMenuSheet, useMinuteClock, useTrashExpanded, useTrashRetention, useTrashTree } from "./SessionTrash";
import { DeleteFolderToTrashShell } from "./DeleteFolderToTrash";
import { folderDeletePlan, liveDrafts, TRASH_ROOT_KEY, TRASH_URGENT_MS, type TrashRow } from "./trashModel";
import { reorderWorkspaceItems } from "./store";
import { DRAFT_DRAG_TYPE } from "./documents/model";
import { sessionDirectoryChoices } from "./sessionDirectoryChoices";
import { newSessionPreset } from "./newSessionPreset";
import { SessionPresetPicker } from "./SessionPresetPicker";
import { WorkspacePicker } from "./WorkspacePicker";
import { AiInstallationPicker, installationCapacity } from "./AiInstallationPicker";
import { useProjectPlacement } from "./useProjectPlacement";
import { SessionSubscription, useSessionSubscription } from "./SessionSubscription";
import { Fragment, lazy, memo, Suspense, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal, flushSync } from "react-dom";
import { Alert, alpha, Box, Button, ButtonBase, Chip, CircularProgress, Divider, IconButton, List, ListItemIcon, ListItemText, ListSubheader, Menu, MenuItem, Skeleton, Stack, TextField, Tooltip, Typography, useMediaQuery, useTheme } from "@mui/material";
import type { Theme } from "@mui/material";
import { Add, Check as CheckIcon, ChevronRight, Circle, CreateNewFolderOutlined, DeleteOutline, BedtimeOutlined, DragIndicator, DriveFileMoveOutlined, DriveFileRenameOutline, ForkRightOutlined, ExpandMore, FolderOpenOutlined, FolderOutlined, InfoOutlined, LabelOutlined, MoreVert, NotificationsOffOutlined, NotificationsOutlined, Refresh as RefreshIcon, Schedule, Settings as SettingsIcon } from "@mui/icons-material";
import { useKeyboardOpen } from "./keyboardInset";
import { DelayedNetworkProgress } from "./NetworkActionFeedback";
import { sessionListProjectLabel, sessionProjectLabel } from "./sessionProject";
import { SessionMachineBadge } from "./SessionMachineBadge";
import { SessionUpdateBadge } from "./SessionUpdateBadge";
import { AccountBadge } from "./AccountBadge";
import { sessionAccountIdentity } from "./accountIdentity";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { requestPickSession } from "./sessionPickRequest";
import { openSessionSettings, requestNewFrom } from "./sessionSettingsOpen";
import { SessionNextMessageBadge } from "./SessionContextCue";
import { useHistorySession, useHistorySessionIds } from "./sessionHistory";
import { isManagedChild } from "./managedCalls";
import { SessionRowTitle } from "./SessionRowTitle";
import { SessionActionBadge, SessionAttentionMenu } from "./SessionAttention";
import { SessionObligationBadge, SessionsSyncedCaption } from "./SessionOfflineBadges";
import { mergeSessionOverview, type SessionInfoPayload, sessionOverviewSections } from "./sessionOverview";
import { useProviderCatalog } from "./providerCatalog";
import { desktopScrollbarSx } from "./desktop/desktopScrollbar";
import { type SessionMeta } from "./protocol";
import { statusColor } from "./SessionStatusDot";
import { bindSessionFolderProject, createSessionFolder, markSessionHydrated, moveSessionFolder, notify, prefetchSessionTail, organizeSessionsByProject, placeSessions, removeSessionFolder, renameSession, renameSessionFolder, reorderSessionFolders, reorderSessions, setSessionFolderDefaults, useStoreSelector } from "./store";
import { type SessionFolder, folderAncestors, normalizeSessionFolderName, sessionFolderById, sessionFolderLocation, unboundProjectLabels } from "./sessionFolders";
import { buildSessionTree, displayedSessionOrder, dropTargetFolder, type FolderActivity, folderIdFromRowKey, foldersRevealing, movedRowKey, projectSessionDrop, rowInsideFolder, type SessionDropProjection, sessionTreeRowKey, type SessionTreeRow } from "./sessionTree";
import { FolderNameShell, InAppRoot, FolderPickerShell, ProjectPickerShell, SESSION_FOLDER_CREATED_EVENT, type SessionFolderCreated, announceSessionFolderCreated, useCollapsedSessionFolders, withFoldersCollapsed } from "./SessionFolderUi";
import { useSessionFoldControl } from "./SessionFoldControl";
import { FolderDefaultsSheet } from "./FolderDefaultsSheet";
import { DirectorySessionsSheet } from "./DirectorySessionsSheet";
import { FOLDER_DEFAULT_FIELDS, resolveFolderDefaults, type FolderDefaultKey, type FolderDefaults } from "./folderDefaults";
import { folderSessionHints } from "./folderSessionHints";
import { createDefaultsChanges, createDefaultsSummary } from "./createDefaultsSummary";
import { FolderCreationDefaults } from "./FolderCreationDefaults";
import { useSheetKeyboardDiagnostics } from "./sheetKeyboardDiagnostics";
import { type SortableDrag, useSortable } from "./useSortable";
import { haptic } from "./haptic";
import { sessionDrawerTargetScroll } from "./mobileDrawerMotion";
import { useNavbarAtBottom } from "./navbarSettings";
import { setTranscriptProjection, captureTranscriptViewportAnchor, useExploreSessionState } from "./explore/exploreStore";
import { DetentSheet } from "@cowboy/app-shell";
import { MobileSessionsDock } from "./MobileSessionsDock";
import { ConfirmSheet, Sheet } from "./Sheet";
import { MobileDecisionActions } from "./MobileDecisionActions";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ShortcutKeycap } from "./ShortcutKeycap";
import { HintTooltip } from "./HintTooltip";
import { DesktopModalKeyHint } from "./desktop/DesktopModalKeyHint";
import { isImeKeyEvent } from "./imeKey";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { DESKTOP_SHORTCUTS } from "./desktop/commands/workspaceShortcuts";
import { useDesktopLeaderOptional } from "./desktop/commands/leaderContext";
import { DESKTOP_MOVE_PICK_EVENT, DESKTOP_SESSION_JUMP_EVENT, desktopPickLabels, publishSessionJumpTargets, sessionJumpLabels } from "./desktop/commands/sessionJump";
import { DesktopShortcut } from "./desktop/commands/DesktopKeycap";
import { fireLabel, fireRel } from "./scheduleTime";
import { desktopImeOwnsKey } from "./desktop/commands/imeShortcut";
import { desktopKeyIntent, installNativeCompositionTracker } from "./desktop/commands/keyIntent";
import { isTextEditingTarget } from "./desktop/commands/shortcut";
import { workspaceCommandKey } from "./desktop/commands/workspaceCommandKey";
import { desktopEmbeddedControlSx, desktopListItemSx } from "./desktop/DesktopEmbeddedControl";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { SessionContextPicker } from "./SessionContextPicker";
import { newUuid } from "./uuid";
import { DesktopModal as DesktopModalShell } from "./desktop/DesktopModal";
import { sessionNotificationsMuted, setSessionNotificationsMuted, useSystemNotificationPreferences } from "./systemNotifications";

export const NO_COLLAPSED_FOLDERS: ReadonlySet<string> = new Set();

export const DesktopSessionShortcut = lazy(async () => {
    const module = await import("./desktop/DesktopSessionShortcut");
    return { default: module.DesktopSessionShortcut };
});

// The Sessions bar loads this module long before the first collapse (see
// DesktopSidebarAction below), but a React.lazy that has never rendered still
// suspends once even for a cached module. The rail is mounted by the collapse itself,
// so that one suspension committed its null fallback: for a frame the hidden
// list and the unresolved rail left no Sessions column at all and the work
// panes reflowed across it. Once the module is here, render the rail directly.
export let desktopPaneCollapseModule: typeof import("./desktop/DesktopPaneCollapse") | null = null;

export const loadDesktopPaneCollapse = async (): Promise<typeof import("./desktop/DesktopPaneCollapse")> => {
    desktopPaneCollapseModule = await import("./desktop/DesktopPaneCollapse");
    return desktopPaneCollapseModule;
};

export const DesktopSidebarAction = lazy(async () => {
    // The Sessions bar renders first; warm the collapsed rail's module with it.
    void loadDesktopPaneCollapse();
    const module = await import("./desktop/DesktopSidebarAction");
    return { default: module.DesktopSidebarAction };
});

// A folder answers "how many of my agents are doing something", which one
// aggregated dot cannot. Each live state gets the session row's own glyph
// (spinner = working, amber dot = needs attention, green dot = ready) with
// its count, grouped in one quiet capsule; dormant sessions only count toward
// the total beside the folder name. Nothing live → no capsule at all, so a
// folder of sleeping sessions reads as calm. Paint-only: safe inside the
// Mobile drawer's swipe compositor (an existing spinner primitive).
export function FolderActivityBadges({
    activity,
    total,
}: {
    activity: FolderActivity;
    total: number;
}): React.JSX.Element | null {
    const parts: { key: string; count: number; glyph: React.ReactNode; label: string }[] = [
        {
            key: "working",
            count: activity.working,
            label: "working",
            glyph: (
                <CircularProgress
                    data-session-activity="true"
                    size="0.875rem"
                    thickness={7}
                    disableShrink
                    color="inherit"
                    sx={{ color: statusColor("busy"), flexShrink: 0 }}
                />
            ),
        },
        {
            key: "attention",
            count: activity.attention,
            label: activity.attention === 1 ? "needs attention" : "need attention",
            glyph: <Circle sx={{ fontSize: glyphSize(8), color: statusColor("interrupted") }} />,
        },
        {
            key: "live",
            count: activity.live,
            label: "ready",
            glyph: <Circle sx={{ fontSize: glyphSize(8), color: statusColor("running") }} />,
        },
    ].filter((part) => part.count > 0);
    if (parts.length === 0) return null;
    const summary = [
        ...parts.map((part) => `${String(part.count)} ${part.label}`),
        `${String(total)} total`,
    ].join(" · ");
    return (
        <Tooltip title={summary} enterDelay={300}>
            <Stack
                direction="row"
                alignItems="center"
                spacing={0.75}
                aria-label={summary}
                data-folder-activity
                sx={{
                    flexShrink: 0,
                    height: "1.375rem",
                    px: 0.75,
                    borderRadius: 99,
                    bgcolor: "action.hover",
                    fontSize: "0.75rem",
                    fontWeight: 650,
                    fontVariantNumeric: "tabular-nums",
                    color: "text.secondary",
                }}
            >
                {parts.map((part) => (
                    <Stack key={part.key} direction="row" alignItems="center" spacing={0.4}>
                        {part.glyph}
                        <Box component="span" sx={{ minWidth: "1ch", textAlign: "center", lineHeight: 1 }}>
                            {part.count}
                        </Box>
                    </Stack>
                ))}
            </Stack>
        </Tooltip>
    );
}

// Scheduled-draft indicator: shown when a session has ≥1 draft with a future
// fire time (`next_schedule_ms` = the soonest). A calm clock glyph in info blue
// (a pending schedule is a notice, not a status/failure — conventions/ui.md §4),
// orthogonal to StatusDot. Tooltip spells out when.
export function ScheduleBadge({ meta }: { meta: SessionMeta }): React.JSX.Element | null {
    const ms = meta.next_schedule_ms;
    if (ms === undefined) return null;
    return <ScheduleBadgeIcon ms={ms} />;
}

// Its own clock: a memoized session row does not re-render merely because
// time passed, so the relative label must refresh itself.
export function ScheduleBadgeIcon({ ms }: { ms: number }): React.JSX.Element {
    const [now, setNow] = useState(() => Date.now());
    useEffect(() => {
        const timer = globalThis.setInterval(() => setNow(Date.now()), 30_000);
        return () => globalThis.clearInterval(timer);
    }, []);
    return (
        <Tooltip title={`定时发送 · ${fireLabel(ms, now)}（${fireRel(ms, now)}）`} enterDelay={300}>
            <Schedule sx={{ fontSize: glyphSize(15), flexShrink: 0, color: "info.main" }} />
        </Tooltip>
    );
}

export function SessionProjectionBadge({
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
            aria-label="Page view"
            color="primary"
            sx={{
                height: "1.65em",
                minWidth: "4.2em",
                flexShrink: 0,
                bgcolor: (theme) => alpha(theme.palette.primary.main, 0.1),
                color: "primary.main",
                fontSize: "0.6875rem",
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

// Sessions-region row commands shared by the Desktop command layer (which
// dispatches them as DOM events on the focused row) and the list's own
// fallback key handler. See docs/sessions-folders.md and desktop/FOCUS.md.
/** The item a Move pick files (FOCUS.md "Move pick"). */
export interface MovePickSubject {
    readonly kind: "session" | "folder" | "draft";
    readonly id: string;
    /** Its Sessions-tree row key. */
    readonly key: string;
    readonly title: string;
}

/** A Move pick letter: loud on purpose, it is the only thing to read. The
 *  typed half of a two-letter label fades. */
export function MovePickLabel({ label, typed }: { label: string; typed: string }): React.JSX.Element {
    const done = typed && label.startsWith(typed) ? typed.length : 0;
    return (
        <Box
            component="kbd"
            data-move-pick-label={label}
            sx={{
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                minWidth: "1.75rem",
                height: "1.75rem",
                px: "0.4rem",
                mr: "0.5rem",
                flexShrink: 0,
                borderRadius: "0.45rem",
                bgcolor: "primary.main",
                color: "primary.contrastText",
                fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
                fontSize: "0.95rem",
                fontWeight: 800,
                letterSpacing: "0.04em",
                textTransform: "uppercase",
                boxShadow: (theme) => `0 0 0 3px ${alpha(theme.palette.primary.main, 0.3)}, 0 2px 8px ${alpha(theme.palette.common.black, 0.35)}`,
            }}
        >
            {done > 0 && <Box component="span" sx={{ opacity: 0.45 }}>{label.slice(0, done)}</Box>}
            {label.slice(done)}
        </Box>
    );
}

/** Darkens everything but the Sessions sidebar while a Move pick waits: a
 *  fixed frame over the sidebar whose huge shadow is the scrim. */
export function SessionsSpotlight({ anchor }: { anchor: React.RefObject<HTMLElement | null> }): React.JSX.Element | null {
    const [rect, setRect] = useState<DOMRect | null>(null);
    useLayoutEffect(() => {
        const update = (): void => setRect(
            anchor.current?.closest<HTMLElement>("[data-desktop-region='sessions.list']")
                ?.getBoundingClientRect() ?? null,
        );
        update();
        globalThis.addEventListener("resize", update);
        return () => globalThis.removeEventListener("resize", update);
    }, [anchor]);
    if (!rect) return null;
    return createPortal(
        <Box
            aria-hidden
            data-desktop-move-spotlight
            sx={{
                position: "fixed",
                left: rect.left,
                top: rect.top,
                width: rect.width,
                height: rect.height,
                pointerEvents: "none",
                zIndex: (theme) => theme.zIndex.modal - 1,
                boxShadow: (theme) => `0 0 0 200vmax ${alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.6 : 0.5)}`,
                animation: "cowboyMovePickIn 140ms ease-out",
                "@keyframes cowboyMovePickIn": { from: { opacity: 0 }, to: { opacity: 1 } },
                "@media (prefers-reduced-motion: reduce)": { animation: "none" },
            }}
        />,
        document.body,
    );
}

export type SessionRowCommand =
    | "open" | "left" | "settings" | "move" | "newFolder" | "rename" | "newSession"
    | "trash" | "preferences";

/** Primary-tint opacity of the open session's row on Desktop. */
export const CURRENT_ROW_FILL = 0.12;

/** Input that hands the Sessions list back to the user while it settles. */
export const SETTLE_RELEASE_EVENTS = ["wheel", "pointerdown", "touchstart", "keydown"] as const;

export const SESSION_ROW_KEY_COMMANDS: Readonly<Record<string, SessionRowCommand>> = {
    l: "open",
    h: "left",
    s: "settings",
    m: "move",
    n: "newFolder",
    i: "rename",
    // The row's actions modal names Rename `R` and Trash `X`; the same
    // letters act on the cursor row directly.
    r: "rename",
    x: "trash",
    // Settings is `␣,` for the app; `,` on a row is that row's settings.
    ",": "preferences",
    // nvim-tree's `a`dd: a new Session in the cursor row's directory.
    a: "newSession",
};

export const SESSION_ROW_EVENTS: Readonly<Record<string, SessionRowCommand>> = {
    "cowboy:desktop-tree-right": "open",
    "cowboy:desktop-tree-left": "left",
    "cowboy:desktop-session-settings": "settings",
    "cowboy:desktop-session-move": "move",
    "cowboy:desktop-folder-new": "newFolder",
    "cowboy:desktop-rename": "rename",
};

/** Re-render one list row only when one of its `deps` changed (`Object.is`). */
export const SessionRowMemo = memo(
    function SessionRowMemo({ render }: { deps: readonly unknown[]; render: () => React.JSX.Element }): React.JSX.Element {
        return render();
    },
    (previous, next) =>
        previous.deps.length === next.deps.length &&
        previous.deps.every((value, index) => Object.is(value, next.deps[index])),
);

export function SessionList({
    sessions,
    drafts,
    activeId,
    onDraftAction,
    onDraftCopy,
    onPick,
    onNew,
    onNewInFolder,
    onClose,
    onOpenSettings,
    onOpenAbout,
    onRequestDelete,
    onRequestInfo,
    onRequestReload,
    onRequestRename,
    loaded,
    desktop,
    mobileDrawer = false,
    mobileDrawerOpen = false,
    phone = false,
    allowNewSession = true,
    desktopHeaderActions = null,
}: {
    sessions: SessionMeta[];
    drafts: readonly DraftMetadata[];
    onDraftAction: (action: WorkspaceDraftAction) => void;
    onDraftCopy: (document: string, session: SessionMeta) => void;
    activeId: string | null;
    onPick: (id: string) => void;
    onNew: () => void;
    /** "New session here" from a folder: the created session is filed there. */
    onNewInFolder?: ((folder: string | null) => void) | undefined;
    onClose?: (() => void) | undefined;
    onOpenSettings?: (() => void) | undefined;
    onOpenAbout?: (() => void) | undefined;
    onRequestDelete: (s: SessionMeta) => void;
    onRequestInfo: (s: SessionMeta) => void;
    onRequestReload: (s: SessionMeta) => void;
    onRequestRename: (s: SessionMeta) => void;
    // True once the first session list has arrived over the WS. Until then the
    // list is genuinely UNKNOWN (not "empty") — distinguishing the two avoids the
    // false "No sessions yet." flash on a reload before the snapshot lands.
    loaded: boolean;
    desktop: boolean;
    mobileDrawer?: boolean;
    mobileDrawerOpen?: boolean;
    phone?: boolean;
    allowNewSession?: boolean;
    /** The docked Desktop sidebar's title bar: Create and fold render there as
     *  view actions (VS Code style) instead of in a row above the list. */
    desktopHeaderActions?: HTMLElement | null;
}): React.JSX.Element {
    // Desktop-only modal list state. Normal mode navigates with j/k; Pin turns
    // the same keys into spatial reorder commands until O/Esc (or opening a
    // session) releases it. This is intentionally local UI state, not synced
    // session metadata.
    const deletingSessionIds = useStoreSelector((snapshot) => snapshot.deletingSessionIds);
    const connected = useStoreSelector((snapshot) => snapshot.connected);
    const { data: accounts } = useSubscriptionAccounts(connected);
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
    const handedOff = useHistorySessionIds();
    // A handed-off session is history; continuing it again would split the work.
    const menuHandedOff = useHistorySession(menuAnchor?.row.id ?? "");
    useSystemNotificationPreferences();
    const menuSessionMuted = menuAnchor
        ? sessionNotificationsMuted(menuAnchor.row.id)
        : false;
    const toggleMenuSessionNotifications = (): void => {
        if (!menuAnchor) return;
        setSessionNotificationsMuted(menuAnchor.row.id, !menuSessionMuted);
        setMenuAnchor(null);
    };
    const sessionMachines = useStoreSelector((snapshot) => snapshot.machines);
    const menuHibernate = menuAnchor
        ? hibernateAvailability(menuAnchor.row, sessionMachines)
        : null;
    const hibernateMenuSession = (): void => {
        if (!menuAnchor || menuHibernate !== "ready") return;
        const session = menuAnchor.row;
        setMenuAnchor(null);
        void hibernateSession(session.id)
            .then(() => documentNotice(`${session.title || "Session"} is hibernating; it resumes when you open it.`))
            .catch((error: Error) => documentNotice(error.message));
    };
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
    const requestRenameFromMenu = (): void => {
        const session = menuAnchor?.row;
        if (!session) return;
        // MUI's open Menu traps focus and restores it to the kebab button on
        // close. Finish both behaviours before the rename opener focuses its
        // real input; otherwise WebKit blurs the keyboard claim inside the same
        // tap and the later Title focus can show a caret without a keyboard.
        flushSync(() => setMenuAnchor(null));
        onRequestRename(session);
    };
    // Drag-to-reorder via the leading grip handle (server-authoritative, synced).
    const byId = new Map(sessions.map((s) => [s.id, s]));
    const listRef = useRef<HTMLUListElement>(null);
    // Desktop P3 prefetch: a mouse resting on a row for 150 ms fetches that
    // session's tail so the switch paints at once (docs/offline-first-sync.md §3).
    const hoverPrefetchTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    // Memoized session rows read these at event time, never from a stale render.
    const rowLive = useRef({ onPick, onDraftCopy, onRequestInfo, activeId });
    rowLive.current = { onPick, onDraftCopy, onRequestInfo, activeId };
    // --- Folders (docs/sessions-folders.md) ---------------------------------
    // The synced folder tree is overlaid on the display-ordered session list.
    // Both surfaces render the same direction — the synced `"order"` array
    // reversed, newest first inside every container — so a session sits in the
    // same place on Desktop and in the Mobile drawer. Collapse state is per
    // device. The sortable runs over the visible tree rows: folder rows have no
    // grip, so only sessions drag, but they still open the gap.
    const sessionFolders = useStoreSelector((snapshot) => snapshot.sessionFolders);
    const [collapsed, setCollapsed] = useCollapsedSessionFolders();
    // Trash (docs/sessions-folders.md#trash): pinned after the tree, closed
    // until toggled; its rows never join the sortable tree.
    const trashFolders = useStoreSelector((snapshot) => snapshot.trashFolders);
    const treeDrafts = useMemo(() => liveDrafts(drafts, trashFolders), [drafts, trashFolders]);
    const [trashExpanded, toggleTrash] = useTrashExpanded();
    const trashRetention = useTrashRetention(connected);
    const trashTree = useTrashTree(drafts, trashExpanded, trashRetention.days);
    // Countdowns tick while Trash is open, or while its closed header shows
    // a deadline under a day.
    const trashNow = useMinuteClock(
        trashExpanded.has(TRASH_ROOT_KEY) ||
            (trashTree.soonest !== null && trashTree.soonest - Date.now() < TRASH_URGENT_MS),
    );
    const [trashRow, setTrashRow] = useState<TrashRow | null>(null);
    const [trashMenu, setTrashMenu] = useState(false);
    const displayedSessions = useMemo(() => displayedSessionOrder(sessions), [sessions]);
    // `␣␣` labels (FOCUS.md "Leader"): letters in flat displayed order.
    const jumpLabels = useMemo(() => sessionJumpLabels(displayedSessions), [displayedSessions]);
    const leader = useDesktopLeaderOptional();
    const sessionLabelsShown = desktop && leader?.armed === true && leader.layer === "sessions";
    // Move pick (FOCUS.md "Move pick"): the subject waits while every folder
    // shows a letter; the leader's "move" layer routes the next keys here.
    const [movePick, setMovePick] = useState<MovePickSubject | null>(null);
    const [movePrefix, setMovePrefix] = useState("");
    const movePicking = desktop && movePick !== null && leader?.armed === true &&
        leader.layer === "move";
    useEffect(() => {
        if (movePicking || movePick === null) return;
        setMovePick(null);
        setMovePrefix("");
    }, [movePicking, movePick]);
    useEffect(() => {
        if (!desktop) return;
        publishSessionJumpTargets(displayedSessions.flatMap((session) => {
            const label = jumpLabels.get(session);
            return label ? [{
                label,
                id: session.id,
                title: session.title,
                detail: sessionProjectLabel(session) ?? "",
                current: session.id === activeId,
            }] : [];
        }));
    }, [activeId, desktop, displayedSessions, jumpLabels]);
    const workspaceOrder = useStoreSelector((snapshot) => snapshot.workspaceOrder);
    // While picking, every folder is open and only folders and the subject
    // remain, so each destination is on screen with its letter.
    const tree = useMemo(
        () => buildSessionTree(
            displayedSessions,
            sessionFolders,
            movePicking ? NO_COLLAPSED_FOLDERS : collapsed,
            treeDrafts,
            workspaceOrder,
        ),
        [collapsed, displayedSessions, sessionFolders, treeDrafts, workspaceOrder, movePicking],
    );
    const listRows = movePicking && movePick
        ? tree.rows.filter((row) => row.kind === "folder" || sessionTreeRowKey(row) === movePick.key)
        : tree.rows;
    const rowKeys = listRows.map(sessionTreeRowKey);
    const rowByKey = new Map(listRows.map((row): [string, SessionTreeRow] => [sessionTreeRowKey(row), row]));
    const moveTargets = ((): readonly { folder: string | null; label: string }[] => {
        if (!movePicking || !movePick) return [];
        const excluded = new Set<string>();
        let current: string | null;
        if (movePick.kind === "folder") {
            excluded.add(movePick.id);
            for (const candidate of sessionFolders.folders) {
                if (folderAncestors(sessionFolders, candidate.id).includes(movePick.id)) excluded.add(candidate.id);
            }
            current = sessionFolderById(sessionFolders, movePick.id)?.parent ?? null;
        } else if (movePick.kind === "draft") {
            current = drafts.find((entry) => entry.id === movePick.id)?.parent_id ?? null;
        } else current = tree.folderOf.get(movePick.id) ?? null;
        const folders = [null, ...listRows.flatMap((row) => row.kind === "folder" ? [row.folder.id] : [])]
            .filter((folder) => folder !== current && !(folder !== null && excluded.has(folder)));
        const labels = desktopPickLabels(folders.length);
        return folders.map((folder, index) => ({ folder, label: labels[index]! }));
    })();
    const moveLabelOf = (folder: string | null): string | undefined =>
        moveTargets.find((target) => target.folder === folder)?.label;
    const [folderMenu, setFolderMenu] = useState<{ folder: SessionFolder; el: HTMLElement } | null>(null);
    const [namePrompt, setNamePrompt] = useState<
        { mode: "create"; parent: string | null } | { mode: "rename"; folder: SessionFolder } | null
    >(null);
    const [movePicker, setMovePicker] = useState<{ kind: "session" | "folder"; id: string } | null>(null);
    const [movedRow, setMovedRow] = useState<string | null>(null);
    const [projectPicker, setProjectPicker] = useState<SessionFolder | null>(null);
    const [defaultsFolder, setDefaultsFolder] = useState<string | null>(null);
    const [selectFolderSessions, setSelectFolderSessions] = useState<string | null>(null);
    const [deleteFolder, setDeleteFolder] = useState<SessionFolder | null>(null);
    const unboundProjects = unboundProjectLabels(sessions, sessionFolders);
    const projectLabels = ((): string[] => {
        const out: string[] = [];
        for (const s of sessions) {
            const label = sessionProjectLabel(s);
            if (label && !out.includes(label)) out.push(label);
        }
        return out;
    })();
    const setFolderCollapsed = (ids: readonly string[], value: boolean): void =>
        setCollapsed((previous) => withFoldersCollapsed(previous, ids, value));
    const focusRow = (key: string): void => {
        requestAnimationFrame(() =>
            listRef.current?.querySelector<HTMLElement>(
                `[data-desktop-item="${CSS.escape(key)}"]`,
            )?.focus({ preventScroll: true })
        );
    };
    const focusPrompt = (): void => {
        requestAnimationFrame(() => {
            const prompt = document.querySelector<HTMLElement>(
                "[data-desktop-region='prompt.composer']",
            );
            const target = prompt?.querySelector<HTMLElement>(
                "[data-vim-command-sink], .cm-content[contenteditable='true']",
            );
            target?.focus({ preventScroll: true });
        });
    };
    // Expand a session's ancestors, so a push, a slot switch or a Desktop
    // region entry never lands on a hidden row.
    const revealSession = (sessionId: string): void =>
        setFolderCollapsed(foldersRevealing(tree, sessionFolders, sessionId), false);
    const revealMovedRow = (key: string, folder: string | null): void => {
        if (folder) setFolderCollapsed([folder, ...folderAncestors(sessionFolders, folder)], false);
        setMovedRow(key);
    };
    const startMovePick = (key: string | null): void => {
        if (!key || !leader) return;
        const folderId = folderIdFromRowKey(key);
        const draft = key.startsWith("draft:") ? drafts.find((entry) => `draft:${entry.id}` === key) : undefined;
        const subject: MovePickSubject | null = folderId
            ? { kind: "folder", id: folderId, key, title: sessionFolderById(sessionFolders, folderId)?.name ?? "Folder" }
            : draft
            ? { kind: "draft", id: draft.id, key, title: draft.title || "Untitled" }
            : byId.has(key)
            ? { kind: "session", id: key, key, title: byId.get(key)!.title }
            : null;
        if (!subject) return;
        setMovePrefix("");
        setMovePick(subject);
        leader.open("move");
    };
    const performMove = (folder: string | null): void => {
        const subject = movePick;
        if (!subject) return;
        setMovePick(null);
        setMovePrefix("");
        leader?.close();
        if (subject.kind === "draft") {
            void draftRepository().document(subject.id).hydrate()
                .then(() => draftRepository().document(subject.id).change({ type: "move", parent_id: folder }))
                .then(() => revealMovedRow(subject.key, folder))
                .catch((error: Error) => documentNotice(error.message));
            return;
        }
        moveItem({ kind: subject.kind, id: subject.id }, folder);
    };
    const onMovePickKey = (detail: { key: string; pending: boolean }): void => {
        if (detail.key === "Backspace") {
            setMovePrefix("");
            detail.pending = true;
            return;
        }
        const typed = movePrefix + detail.key.toLowerCase();
        const hit = moveTargets.find((target) => target.label === typed);
        if (hit) {
            performMove(hit.folder);
            return;
        }
        // A prefix waits for its second letter; any other key is ignored so
        // a slip never files the item somewhere unintended. Esc leaves.
        if (moveTargets.some((target) => target.label.startsWith(typed))) setMovePrefix(typed);
        detail.pending = true;
    };
    const onMovePickKeyRef = useRef(onMovePickKey);
    onMovePickKeyRef.current = onMovePickKey;
    const moveItem = (item: { kind: "session" | "folder"; id: string }, folder: string | null): void => {
        const current = item.kind === "folder"
            ? sessionFolderById(sessionFolders, item.id)?.parent ?? null
            : tree.folderOf.get(item.id) ?? null;
        if (folder === current) return;
        if (item.kind === "folder") moveSessionFolder(item.id, folder);
        else placeSessions([item.id], folder);
        revealMovedRow(item.kind === "folder" ? `folder:${item.id}` : item.id, folder);
    };
    useLayoutEffect(() => {
        if (!movedRow) return undefined;
        const frame = requestAnimationFrame(() => {
            const list = listRef.current;
            const item = list?.querySelector<HTMLElement>(
                `[data-desktop-item="${CSS.escape(movedRow)}"]`,
            );
            if (!list || !item) return;
            list.scrollTop = sessionDrawerTargetScroll({
                currentScroll: list.scrollTop,
                viewportHeight: list.clientHeight,
                contentHeight: list.scrollHeight,
                itemTop: item.getBoundingClientRect().top - list.getBoundingClientRect().top + list.scrollTop,
                itemHeight: item.offsetHeight,
            });
            if (desktop) item.focus({ preventScroll: true });
            setMovedRow(null);
        });
        return () => cancelAnimationFrame(frame);
    }, [movedRow, tree, desktop]);
    const fold = useSessionFoldControl({
        listRef,
        tree,
        folders: sessionFolders,
        collapsed,
        setCollapsed: (next) => setCollapsed(() => next),
        activeId,
        // The Mobile drawer's last rows sit under the floating action islands.
        bottomInset: mobileDrawer ? 84 : 0,
        onLocate: setMovedRow,
    });
    // A folder made in Create lands in view, like a moved one.
    const revealMovedRowRef = useRef(revealMovedRow);
    revealMovedRowRef.current = revealMovedRow;
    useEffect(() => {
        const onCreated = (event: Event): void => {
            const { id, parent } = (event as CustomEvent<SessionFolderCreated>).detail;
            revealMovedRowRef.current(`folder:${id}`, parent);
        };
        globalThis.addEventListener(SESSION_FOLDER_CREATED_EVENT, onCreated);
        return () => globalThis.removeEventListener(SESSION_FOLDER_CREATED_EVENT, onCreated);
    }, []);
    const foldersHydrated = sessionFolders.folders.length > 0;
    useEffect(() => {
        if (activeId) revealSession(activeId);
        // Only a new active session (or the folder tree's first arrival after a
        // reload) triggers a reveal; a later manual collapse of its folder
        // must stick.
    }, [activeId, foldersHydrated]);
    // A drop (or an Order-mode step) hands back the visible row keys in their
    // new order. A moved session lands in the container of the row above it
    // (a folder header files into that folder) and the flat session order is
    // re-submitted; a moved folder permutes its siblings only.
    const applyRowOrder = (
        order: string[],
        movedKey: string | null,
        // A pointer drag passes its projected container (vertical slot plus
        // horizontal intent); Order mode infers it from the row above.
        projectedFolder?: string | null,
    ): void => {
        const moved = movedKey ? rowByKey.get(movedKey) : undefined;
        if (!moved || moved.kind === "empty") return;
        if (moved.kind === "folder") {
            const siblings = order
                .map((key) => rowByKey.get(key))
                .filter((row): row is SessionTreeRow & { kind: "folder" } =>
                    row?.kind === "folder" && row.folder.parent === moved.folder.parent
                )
                .map((row) => row.folder.id);
            reorderSessionFolders(moved.folder.parent, siblings);
            return;
        }
        const index = order.indexOf(movedKey ?? "");
        const others = order
            .filter((key) => key !== movedKey)
            .map((key) => rowByKey.get(key))
            .filter((row): row is SessionTreeRow => row !== undefined);
        const target = projectedFolder !== undefined
            ? projectedFolder
            : dropTargetFolder(others, index);
        const workspaceOrder = order.filter((key) => {
            const row = rowByKey.get(key);
            return row?.kind === "draft" || row?.kind === "session";
        }).map((key) => key.startsWith("draft:") ? key : `session:${key}`);
        reorderWorkspaceItems(workspaceOrder);
        if (moved.kind === "draft") {
            if (target !== moved.draft.parent_id) {
                void draftRepository().document(moved.draft.id).hydrate()
                    .then(() => draftRepository().document(moved.draft.id).change({ type: "move", parent_id: target }))
                    .then(() => revealMovedRow(movedKey!, target))
                    .catch((error: Error) => documentNotice(error.message));
            }
            return;
        }
        const refiled = target !== (tree.folderOf.get(moved.session.id) ?? null);
        if (refiled) {
            placeSessions([moved.session.id], target);
            revealMovedRow(moved.session.id, target);
        }
        const reordered = order.some((key, i) => key !== rowKeys[i]);
        if (!reordered) return;
        const sessionOrder = order
            .map((key) => rowByKey.get(key))
            .filter((row): row is SessionTreeRow & { kind: "session" } => row?.kind === "session")
            .map((row) => row.session.id);
        reorderSessions(displayedSessionOrder(sessionOrder));
    };
    const applyRowOrderRef = useRef(applyRowOrder);
    applyRowOrderRef.current = applyRowOrder;
    // Drag-into-folder (Obsidian's explorer, adapted to an ordered list): the
    // vertical slot bounds where a session may land, horizontal movement picks
    // the nesting depth inside those bounds, and the target folder lights up
    // with its whole body while the dragged row slides to the indent it will
    // take. One indent step of finger travel is a little wider than the
    // visual indent so a vertical drag never wobbles between depths.
    const finePointer = useMediaQuery("(pointer: fine) and (hover: hover)");
    const indentPx = finePointer || mobileDrawer ? 16 : 20;
    const [dragIndentPx, setDragIndentPx] = useState(0);
    const projectDrag = (drag: SortableDrag): SessionDropProjection | null => {
        const dragged = rowByKey.get(drag.id);
        if (dragged?.kind !== "session" && dragged?.kind !== "draft") return null;
        const others = tree.rows.filter((row) => sessionTreeRowKey(row) !== drag.id);
        return projectSessionDrop(others, drag.targetIndex, dragged.depth, drag.depthSteps);
    };
    const sortable = useSortable({
        ids: rowKeys,
        itemDrop: (id) => id.startsWith("draft:"),
        onReorder: (order): void =>
            applyRowOrder(order, sortable.draggingId ?? movedRowKey(rowKeys, order)),
        onDrop: (order, drag): void => {
            const over = drag.overId ? rowByKey.get(drag.overId) : undefined;
            if (drag.id.startsWith("draft:") && over?.kind === "session") {
                onDraftCopy(drag.id.slice(6), over.session);
                return;
            }
            if (drag.id.startsWith("draft:") && over?.kind === "folder") {
                applyRowOrder(order, drag.id, over.folder.id);
                return;
            }
            const projection = projectDrag(drag);
            applyRowOrder(order, drag.id, projection ? projection.folder : undefined);
        },
        scrollContainer: () => listRef.current,
        horizontalStep: indentPx + 12,
        dragOffsetX: dragIndentPx,
        optimisticReorder: false,
    });
    const copyTarget = sortable.drag?.id.startsWith("draft:") && sortable.drag.overId
        ? rowByKey.get(sortable.drag.overId) : undefined;
    const dropProjection = sortable.drag ? projectDrag(sortable.drag) : null;
    const draggedDepth = sortable.drag
        ? (rowByKey.get(sortable.drag.id)?.depth ?? 0)
        : 0;
    const nextDragIndentPx = dropProjection
        ? (dropProjection.depth - draggedDepth) * indentPx
        : 0;
    useEffect(() => {
        setDragIndentPx(nextDragIndentPx);
        // A change of target container is a distinct, felt step.
        if (sortable.drag) haptic(8);
    }, [nextDragIndentPx, dropProjection?.folder]);
    const dropFolderName = dropProjection?.folder
        ? sessionFolderById(sessionFolders, dropProjection.folder)?.name ?? null
        : null;
    // Paint-only drag feedback per row: the target folder's header and body.
    const dropHighlight = (row: SessionTreeRow, key: string): "header" | "body" | null => {
        const target = dropProjection?.folder;
        if (!target || key === sortable.drag?.id) return null;
        if (row.kind === "folder" && row.folder.id === target) return "header";
        return rowInsideFolder(row, target, sessionFolders) ? "body" : null;
    };
    const runRowCommand = (
        command: SessionRowCommand,
        rowKey: string,
        el: HTMLElement,
        detail: { toggle?: boolean } = {},
    ): void => {
        if (rowKey.startsWith("trash:")) {
            // Trash rows: Enter/l toggles a folder or opens an entry's actions,
            // h closes, s opens actions. Nothing else applies to deleted items.
            const row = trashTree.rows.find((candidate) => candidate.key === rowKey);
            if (rowKey === TRASH_ROOT_KEY) {
                if (command === "open") toggleTrash(TRASH_ROOT_KEY, detail.toggle ? undefined : true);
                else if (command === "left") toggleTrash(TRASH_ROOT_KEY, false);
                else if (command === "settings") setTrashMenu(true);
            } else if (row?.kind === "trash-folder" && (command === "open" || command === "left")) {
                if (command === "left" && !row.expanded) focusRow(TRASH_ROOT_KEY);
                else toggleTrash(rowKey, command === "left" ? false : detail.toggle ? undefined : true);
            } else if (row && (command === "open" || command === "settings")) setTrashRow(row);
            else if (row && command === "left") focusRow(TRASH_ROOT_KEY);
            return;
        }
        const folderId = folderIdFromRowKey(rowKey);
        const folder = folderId ? sessionFolderById(sessionFolders, folderId) : undefined;
        const session = folderId ? undefined : byId.get(rowKey);
        const draft = drafts.find((entry) => `draft:${entry.id}` === rowKey);
        if (draft) {
            if (command === "open") { onPick(rowKey); focusPrompt(); }
            else if (command === "left") { if (draft.parent_id) focusRow(`folder:${draft.parent_id}`); }
            else if (command === "newFolder") setNamePrompt({ mode: "create", parent: draft.parent_id });
            else if (command === "newSession") onNewInFolder?.(draft.parent_id ?? null);
            else if (command === "move" && desktop) startMovePick(rowKey);
            else onDraftAction({ draft, action: command === "settings" || command === "preferences" ? "menu" : command });
            return;
        }
        switch (command) {
            case "open":
                if (folder) {
                    const isCollapsed = collapsed.has(folder.id);
                    setFolderCollapsed([folder.id], detail.toggle ? !isCollapsed : false);
                } else if (session) {
                    setPinned(false);
                    onPick(session.id);
                    focusPrompt();
                }
                return;
            case "left":
                if (folder) {
                    if (!collapsed.has(folder.id)) setFolderCollapsed([folder.id], true);
                    else if (folder.parent) focusRow(`folder:${folder.parent}`);
                    else setFolderCollapsed(sessionFolders.folders.map((f) => f.id), true);
                } else if (session) {
                    const parent = tree.folderOf.get(session.id) ?? null;
                    if (parent) focusRow(`folder:${parent}`);
                }
                return;
            case "settings":
                if (folder) setFolderMenu({ folder, el });
                else if (session) setMenuAnchor({ row: session, el });
                return;
            case "move":
                if (desktop) startMovePick(rowKey);
                else if (folder) setMovePicker({ kind: "folder", id: folder.id });
                else if (session) setMovePicker({ kind: "session", id: session.id });
                return;
            case "newFolder":
                setNamePrompt({
                    mode: "create",
                    parent: folder ? folder.id : session ? tree.folderOf.get(session.id) ?? null : null,
                });
                return;
            case "rename":
                if (folder) setNamePrompt({ mode: "rename", folder });
                else if (session) onRequestRename(session);
                return;
            case "newSession":
                onNewInFolder?.(folder ? folder.id : session ? tree.folderOf.get(session.id) ?? null : null);
                return;
            case "trash": {
                // Both folders and sessions confirm before moving to Trash.
                if (folder) {
                    setDeleteFolder(folder);
                    return;
                }
                if (!session) return;
                const visible = [...(listRef.current?.querySelectorAll<HTMLElement>("[data-desktop-item]") ?? [])]
                    .filter((candidate) => candidate.offsetParent !== null);
                const index = visible.indexOf(el);
                const neighbour = (visible[index + 1] ?? visible[index - 1])?.dataset.desktopItem;
                onRequestDelete(session);
                if (neighbour) {
                    window.setTimeout(() => {
                        if (!el.isConnected && !document.querySelector("[role=\"dialog\"]")) focusRow(neighbour);
                    }, 300);
                }
                return;
            }
            case "preferences":
                if (folder) setDefaultsFolder(folder.id);
                else if (session) {
                    requestPickSession(session.id, session);
                    openSessionSettings("agent", session.id);
                }
                return;
        }
    };
    const runRowCommandRef = useRef(runRowCommand);
    runRowCommandRef.current = runRowCommand;
    const selectSessionLabel = (label: string): boolean => {
        const session = displayedSessions.find((candidate) => jumpLabels.get(candidate) === label);
        if (!session || session.id === activeId) return false;
        revealSession(session.id);
        setPinned(false);
        onPick(session.id);
        focusRow(session.id);
        return true;
    };
    const selectSessionLabelRef = useRef(selectSessionLabel);
    selectSessionLabelRef.current = selectSessionLabel;
    const runFoldersAction = (action: string, rowKey: string | null): void => {
        if (action === "move" && desktop) {
            startMovePick(rowKey ?? activeId);
            return;
        }
        if (action === "move") {
            const key = rowKey ?? activeId;
            const draft = drafts.find((entry) => `draft:${entry.id}` === key);
            if (draft) { onDraftAction({ draft, action: "move" }); return; }
        }
        const currentFolder = ((): string | null => {
            if (!rowKey) return activeId ? tree.folderOf.get(activeId) ?? null : null;
            return folderIdFromRowKey(rowKey) ?? tree.folderOf.get(rowKey) ?? null;
        })();
        switch (action) {
            case "newFolder":
                setNamePrompt({ mode: "create", parent: currentFolder });
                return;
            case "newSession":
                // `␣N` and Create: the directory you are working in, like a
                // file explorer's New File. The form shows it and can change it.
                if (onNewInFolder) onNewInFolder(currentFolder);
                else onNew();
                return;
            case "organize":
                if (organizeSessionsByProject() === 0) notify("Every project already has a folder.");
                return;
            case "collapseAll":
                setFolderCollapsed(sessionFolders.folders.map((f) => f.id), true);
                return;
            case "expandAll":
                setFolderCollapsed(sessionFolders.folders.map((f) => f.id), false);
                return;
            case "fold":
                fold.run();
                return;
            case "reveal":
                if (activeId) {
                    revealSession(activeId);
                    focusRow(activeId);
                }
                return;
            case "move":
                if (rowKey) {
                    const folderId = folderIdFromRowKey(rowKey);
                    setMovePicker(folderId ? { kind: "folder", id: folderId } : { kind: "session", id: rowKey });
                } else if (activeId) setMovePicker({ kind: "session", id: activeId });
                return;
        }
    };
    const runFoldersActionRef = useRef(runFoldersAction);
    runFoldersActionRef.current = runFoldersAction;
    const openFolderName = (prompt: NonNullable<typeof namePrompt>): void => {
        // Mount the name input inside the opening tap so iOS raises the keyboard
        // (the RenameSessionShell contract).
        flushSync(() => {
            setFolderMenu(null);
            setMovePicker(null);
            setNamePrompt(prompt);
        });
    };
    // Each time the list comes into view — the Mobile drawer opening, or the
    // in-flow rail mounting after a reload — bring the current session into
    // the reading band once. Later picks never move the list under the pointer.
    const listShown = !mobileDrawer || mobileDrawerOpen;
    const positionedShownRef = useRef(false);
    useLayoutEffect(() => {
        if (!listShown) {
            positionedShownRef.current = false;
            return undefined;
        }
        if (positionedShownRef.current || !loaded) return undefined;
        positionedShownRef.current = true;
        const place = (): void => {
            const list = listRef.current;
            const current = activeId
                ? list?.querySelector<HTMLElement>(
                    `[data-desktop-item="${CSS.escape(activeId)}"]`,
                )
                : null;
            if (!list || !current) return;
            list.scrollTop = sessionDrawerTargetScroll({
                currentScroll: list.scrollTop,
                viewportHeight: list.clientHeight,
                contentHeight: list.scrollHeight,
                itemTop: current.getBoundingClientRect().top - list.getBoundingClientRect().top + list.scrollTop,
                itemHeight: current.offsetHeight,
            });
        };
        let frame = requestAnimationFrame(() => {
            frame = requestAnimationFrame(place);
        });
        // A reload keeps filling the rail after the first paint (late rows,
        // drafts, badges), which pushes the placed row back out of view.
        // Follow those changes briefly, and stop as soon as the user takes
        // the list. The drawer opens onto a settled list and needs none.
        const list = listRef.current;
        if (mobileDrawer || !list) return () => cancelAnimationFrame(frame);
        const settle = new MutationObserver(() => {
            cancelAnimationFrame(frame);
            frame = requestAnimationFrame(place);
        });
        const release = (): void => {
            settle.disconnect();
            clearTimeout(timer);
            for (const type of SETTLE_RELEASE_EVENTS) list.removeEventListener(type, release);
        };
        const timer = setTimeout(release, 3000);
        settle.observe(list, { childList: true, subtree: true });
        for (const type of SETTLE_RELEASE_EVENTS) list.addEventListener(type, release, { passive: true });
        return () => {
            cancelAnimationFrame(frame);
            release();
        };
    }, [activeId, loaded, listShown, mobileDrawer]);
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
            applyRowOrderRef.current(order, id);
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
        // Row commands arrive from the Desktop command layer as DOM events on
        // the focused row (h/l/s/m/n/i); folder-wide actions and Alt+digit slot
        // switches arrive on the list itself.
        const onRowCommand = (event: Event): void => {
            const command = SESSION_ROW_EVENTS[event.type];
            const item = event.target instanceof Element
                ? event.target.closest<HTMLElement>("[data-desktop-item]")
                : null;
            const key = item?.dataset.desktopItem;
            if (!command || !item || !key) return;
            event.preventDefault();
            runRowCommandRef.current(
                command,
                key,
                item,
                (event as CustomEvent<{ toggle?: boolean }>).detail ?? {},
            );
        };
        const onFolders = (event: Event): void => {
            const detail = (event as CustomEvent<{ action?: string; row?: string | null }>).detail;
            if (!detail?.action) return;
            event.preventDefault();
            runFoldersActionRef.current(detail.action, detail.row ?? null);
        };
        const onJump = (event: Event): void => {
            const label = (event as CustomEvent<{ label?: string }>).detail?.label;
            if (label !== undefined && selectSessionLabelRef.current(label)) event.preventDefault();
        };
        for (const type of Object.keys(SESSION_ROW_EVENTS)) list.addEventListener(type, onRowCommand);
        const onMovePickEvent = (event: Event): void => {
            const detail = (event as CustomEvent<{ key: string; pending: boolean }>).detail;
            if (detail) onMovePickKeyRef.current(detail);
        };
        list.addEventListener("cowboy:desktop-folders", onFolders);
        list.addEventListener(DESKTOP_SESSION_JUMP_EVENT, onJump);
        list.addEventListener(DESKTOP_MOVE_PICK_EVENT, onMovePickEvent);
        return () => {
            for (const type of Object.keys(SESSION_ROW_EVENTS)) list.removeEventListener(type, onRowCommand);
            list.removeEventListener("cowboy:desktop-folders", onFolders);
            list.removeEventListener(DESKTOP_SESSION_JUMP_EVENT, onJump);
            list.removeEventListener(DESKTOP_MOVE_PICK_EVENT, onMovePickEvent);
        };
    }, [desktop]);
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
            !desktop || desktopImeOwnsKey(event.nativeEvent) ||
            event.metaKey || event.ctrlKey || event.altKey || event.shiftKey ||
            event.repeat
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
        const rowKey = row.dataset.desktopItem;
        const command = key === "Enter" ? "open" : SESSION_ROW_KEY_COMMANDS[key];
        if (!rowKey || !command) return;
        // Let a focused grip or kebab retain native Enter activation.
        if (key === "Enter" && event.target !== row) return;
        event.preventDefault();
        event.stopPropagation();
        runRowCommand(command, rowKey, row, { toggle: key === "Enter" });
    };
    // Rows are flush with the rail: the grip and kebab glyphs already carry
    // their own inset inside their hit boxes, so an outer gutter and a
    // rounded capsule only cost title width. Tree rows indent by MARGIN, not
    // padding, so a nested row's own selection, hover and drop material
    // always stays inside its folder: it starts ON the parent's guide, which
    // is then the row's own leading edge, and never paints across it. A row
    // that started one pixel later left the pale guide beside a tinted fill,
    // which read as a gap. One step is therefore exactly the guide's offset:
    // base padding + half the chevron box.
    const ROW_BASE_PL = mobileDrawer ? "0px" : "max(env(safe-area-inset-left), 4px)";
    const ROW_END_PR = mobileDrawer ? 0 : "max(env(safe-area-inset-right), 4px)";
    const INDENT_STEP = `calc(${ROW_BASE_PL} + 14px)`;
    const INDENT_FINE_STEP = 12;
    const rowIndentSx = (depth: number, extraPl = 0, extraFinePl = extraPl) => ({
        pl: `calc(${ROW_BASE_PL} + ${String(extraPl)}px)`,
        ml: `calc(${String(depth)} * ${INDENT_STEP})`,
        "@media (pointer: fine) and (hover: hover)": {
            pl: `${String(extraFinePl)}px`,
            ml: `${String(depth * INDENT_FINE_STEP)}px`,
        },
    });
    // Flush Desktop rows drop the shared capsule: no radius, and no border
    // whose left and right edges would sit on the rail's own edges. The
    // keyboard ring moves inside the row, where the scroller cannot clip it.
    const flushRowSx = desktop
        ? {
            borderWidth: 0,
            borderRadius: 0,
            "&.Mui-focusVisible": {
                boxShadow: (t: Theme) => `inset 0 0 0 1px ${t.palette.primary.main}`,
            },
        }
        : {};
    // Obsidian-style indent guides: one hairline per ancestor level, centred
    // under that ancestor's chevron (28px touch / 24px fine box after the base
    // padding). The pseudo-element reaches back over the row's indent margin;
    // rows touch, so consecutive rows join into one continuous line and a
    // folder's body is visibly fenced off. Paint-only.
    const treeGuideSx = (depth: number) => {
        if (depth === 0) return {};
        const levels = Array.from({ length: depth }, (_, level) => level);
        return {
            "&::before": {
                content: '""',
                position: "absolute",
                top: 0,
                bottom: 0,
                left: `calc(-1 * ${String(depth)} * ${INDENT_STEP})`,
                right: 0,
                pointerEvents: "none",
                backgroundImage: (t: Theme) =>
                    levels.map(() => `linear-gradient(${t.palette.divider}, ${t.palette.divider})`).join(", "),
                backgroundSize: levels.map(() => "1px 100%").join(", "),
                backgroundRepeat: "no-repeat",
                backgroundPosition: levels
                    .map((level) => `calc(${String(level)} * ${INDENT_STEP} + ${ROW_BASE_PL} + 14px) 0`)
                    .join(", "),
                "@media (pointer: fine) and (hover: hover)": {
                    left: `${String(-depth * INDENT_FINE_STEP)}px`,
                    backgroundPosition: levels.map((level) => `${String(level * INDENT_FINE_STEP + 12)}px 0`).join(", "),
                },
            },
        };
    };
    const dropHighlightSx = (highlight: "header" | "body" | null) =>
        highlight === "header"
            ? {
                bgcolor: (t: Theme) => alpha(t.palette.primary.main, 0.16),
                boxShadow: (t: Theme) => `inset 0 0 0 1px ${alpha(t.palette.primary.main, 0.55)}`,
            }
            : highlight === "body"
            ? { bgcolor: (t: Theme) => alpha(t.palette.primary.main, 0.07) }
            : {};
    const folderMenuFolder = folderMenu?.folder ?? null;
    const folderMenuCount = folderMenuFolder
        ? tree.rows.find((r) => r.kind === "folder" && r.folder.id === folderMenuFolder.id)
        : undefined;
    const deleteFolderPlan = useMemo(
        // Captured when the dialog opens: a live recount would move the
        // checkboxes under the reader.
        () => deleteFolder ? folderDeletePlan(deleteFolder.id, sessions, sessionFolders, treeDrafts) : null,
        [deleteFolder],
    );
    const trashRowSx = (depth: number) => ({
        ...(desktop && desktopListItemSx()),
        ...flushRowSx,
        ...treeGuideSx(depth),
        ...rowIndentSx(depth),
        pr: ROW_END_PR,
        "@media (pointer: fine) and (hover: hover)": {
            ...rowIndentSx(depth)["@media (pointer: fine) and (hover: hover)"],
            pr: 0,
            py: 0.25,
        },
    });
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
            {copyTarget?.kind === "session" && <Box role="status" sx={{ px: 2, py: 0.75, fontSize: "0.75rem", color: "primary.main" }}>
                Add to {copyTarget.session.title} · Original kept
            </Box>}
            {movePicking && movePick && <SessionsSpotlight anchor={listRef} />}
            {/* Desktop keeps this row as low as the top bar's controls: the
                list below is what the sidebar is for. */}
            {desktopHeaderActions && createPortal(
                <Suspense fallback={null}>
                    {allowNewSession && (
                        <DesktopSidebarAction
                            data-desktop-new-session="true"
                            label="Create"
                            detail="Session, Draft or Folder · in the current directory"
                            shortcut={DESKTOP_SHORTCUTS.newSession}
                            disabled={movePicking}
                            onClick={(): void => runFoldersAction("newSession", null)}
                        >
                            <Add />
                        </DesktopSidebarAction>
                    )}
                    {fold.action && (
                        <DesktopSidebarAction
                            data-desktop-sessions-fold="true"
                            label={fold.label}
                            shortcut={DESKTOP_SHORTCUTS.sessionsFold}
                            disabled={movePicking}
                            onClick={fold.run}
                        >
                            {fold.icon}
                        </DesktopSidebarAction>
                    )}
                </Suspense>,
                desktopHeaderActions,
            )}
            {!mobileDrawer && (!desktopHeaderActions || movePicking || pinned) && <Box sx={{ p: 1, ...(desktop && { py: 0.5 }) }}>
                {movePicking && movePick && (
                    <Box
                        role="status"
                        data-move-pick-banner
                        sx={{
                            mb: 0.75,
                            p: 1,
                            borderRadius: 1.25,
                            border: 1.5,
                            borderColor: "primary.main",
                            bgcolor: (theme) => alpha(theme.palette.primary.main, 0.1),
                        }}
                    >
                        <Typography variant="body2" noWrap sx={{ fontWeight: 700 }}>
                            Move “{movePick.title}” to…
                        </Typography>
                        {moveLabelOf(null) !== undefined && (
                            <ButtonBase
                                onPointerDown={(event): void => {
                                    event.preventDefault();
                                    performMove(null);
                                }}
                                sx={{ mt: 0.75, width: "100%", justifyContent: "flex-start", borderRadius: 1, py: 0.25 }}
                            >
                                <MovePickLabel label={moveLabelOf(null)!} typed={movePrefix} />
                                <Typography variant="body2" sx={{ fontWeight: 600 }}>Top level</Typography>
                            </ButtonBase>
                        )}
                        <Stack direction="row" alignItems="center" spacing={0.75} sx={{ mt: 0.75, color: "text.secondary" }}>
                            <ShortcutKeycap keyLabel="Esc" availability="active" accent />
                            <Typography variant="caption">cancel</Typography>
                            <Box sx={{ flex: 1 }} />
                            <Typography variant="caption">press a folder’s letter</Typography>
                        </Stack>
                    </Box>
                )}
                {!desktopHeaderActions && <Stack
                    direction="row"
                    spacing={0.75}
                    alignItems="stretch"
                    justifyContent="flex-end"
                    sx={movePicking ? { opacity: 0.35, pointerEvents: "none" } : undefined}
                >
                {allowNewSession && <Button
                    data-desktop-new-session={desktop ? "true" : undefined}
                    fullWidth
                    variant="outlined"
                    startIcon={<Add />}
                    onClick={onNew}
                    sx={desktop ? {
                        ...desktopEmbeddedControlSx(),
                        minHeight: "2.25rem",
                        py: 0,
                        fontSize: "0.875rem",
                        minWidth: 0,
                        flexWrap: "wrap",
                        gap: "0.25rem",
                        textTransform: "none",
                        letterSpacing: "normal",
                    } : undefined}
                >
                    Create
                    {desktop && <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.newSession} quiet />}
                </Button>}
                {fold.action && <Tooltip title={fold.label} describeChild>
                    <IconButton
                        aria-label={fold.label}
                        aria-keyshortcuts={desktop ? DESKTOP_SHORTCUTS.sessionsFold : undefined}
                        onClick={fold.run}
                        sx={{
                            ...(desktop && desktopEmbeddedControlSx()),
                            // Desktop carries its `␣SZ` slot beside the glyph.
                            width: desktop ? "auto" : 48,
                            minWidth: desktop ? "2.25rem" : undefined,
                            px: desktop ? "0.5rem" : undefined,
                            py: desktop ? 0 : undefined,
                            gap: desktop ? "0.375rem" : undefined,
                            minHeight: desktop ? "2.25rem" : 48,
                            // The theme sizes every icon button as a 44px
                            // target; this one sets the row's height.
                            height: desktop ? "2.25rem" : undefined,
                            borderRadius: 1.25,
                            flexShrink: 0,
                            fontSize: "1.1rem",
                        }}
                    >
                        {fold.icon}
                        {desktop && <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.sessionsFold} quiet />}
                    </IconButton>
                </Tooltip>}
                </Stack>}
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
            <SessionAttentionMenu sessions={sessions} onPick={onPick} />
            <List
                dense
                ref={listRef}
                data-desktop-fold-action={desktop ? fold.action ?? undefined : undefined}
                data-mobile-overflow-layer={mobileDrawer ? "true" : undefined}
                onKeyDownCapture={onDesktopListKeyDown}
                data-move-picking={movePicking ? "true" : undefined}
                sx={{
                    ...(movePicking && movePick ? {
                        "& [data-move-pick-dim='true']": { opacity: 0.4 },
                        [`& [data-desktop-item="${CSS.escape(movePick.key)}"]`]: {
                            outline: (theme: Theme) => `2px dashed ${theme.palette.primary.main}`,
                            outlineOffset: -2,
                        },
                    } : {}),
                    flex: 1,
                    overflowY: "auto",
                    pt: mobileDrawer ? "calc(var(--cowboy-system-top-clearance, 0px) + 20px)" : undefined,
                    scrollPaddingTop: mobileDrawer ? "calc(var(--cowboy-system-top-clearance, 0px) + 20px)" : undefined,
                    overflowX: "hidden",
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
                    // Touch keeps 44px grip and kebab targets, but the grip
                    // overhangs its 24px slot, which is just the glyph: the
                    // dots sit beside the row's leading edge (its folder's
                    // guide) with no gutter of their own. The kebab stays
                    // inside the row: an overhang past the scroller's end edge
                    // would let focus scroll the list sideways.
                    "& .cowboy-session-grip": { mx: "-10px" },
                    "& .cowboy-session-grip-slot": { width: 24 },
                    "& .cowboy-session-actions": { ml: 0 },
                    // The folder chevron is decorative (the whole row toggles),
                    // so it needs no touch-sized box; tree indentation and
                    // guides are measured from this narrower width.
                    "& .cowboy-folder-chevron": { width: 28, mx: 0 },
                    // Fine-pointer desktops do not need phone-sized 44px controls
                    // in every row. Keep the generous targets for touch/tablet,
                    // while fitting more sessions without making the rail noisy.
                    "@media (pointer: fine) and (hover: hover)": {
                        py: 0.5,
                        // 24px is the glyph plus a hover halo; the 32px height
                        // keeps a comfortable vertical target.
                        // desktopSize: the Desktop rail follows the global
                        // font size; other fine-pointer surfaces keep 16px units.
                        "& .cowboy-session-grip, & .cowboy-session-actions": {
                            width: desktopSize(24),
                            minWidth: 0,
                            height: desktopSize(32),
                            minHeight: 0,
                            borderRadius: "6px",
                        },
                        // The grip overhangs a 16px column, so its dots start
                        // 5px from the row's leading edge; the slot's height
                        // stops the touch box setting the row height.
                        "& .cowboy-session-grip": { mx: desktopSize(-4) },
                        "& .cowboy-session-grip-slot": { width: desktopSize(16), height: desktopSize(32) },
                        "& .cowboy-session-actions": { ml: 0.25, mr: 0 },
                        "& .cowboy-folder-chevron": { width: 24, mx: 0 },
                        "& .cowboy-session-grip .MuiSvgIcon-root, & .cowboy-session-actions .MuiSvgIcon-root": {
                            fontSize: "1.125rem",
                        },
                    },
                    // Desktop reorders from the keyboard (`O`); the grip is only
                    // the pointer's drag handle. Rows sit flush with the window
                    // edge, so a grip shown at rest with its own hover halo lit
                    // up row after row as the pointer swept past. It now fades
                    // in after a short dwell on the row, at once on the
                    // keyboard cursor row and in Order mode, without a halo.
                    // Folder chevrons share the class and stay visible.
                    ...(desktop && {
                        "& .cowboy-session-grip:not(.cowboy-folder-chevron)": {
                            opacity: pinned ? 1 : 0,
                            transition: "opacity 120ms ease",
                            "&:hover": { bgcolor: "transparent", color: "text.secondary" },
                        },
                        "& [data-desktop-item]:hover .cowboy-session-grip:not(.cowboy-folder-chevron)": {
                            opacity: 1,
                            transitionDelay: "250ms",
                        },
                        "& [data-desktop-item]:focus .cowboy-session-grip:not(.cowboy-folder-chevron), & .cowboy-session-grip:not(.cowboy-folder-chevron):focus-visible": {
                            opacity: 1,
                            transitionDelay: "0ms",
                        },
                        // The trailing actions button sits beside the pane
                        // splitter: the same dwell keeps a pointer heading for
                        // the divider from lighting it. `S` opens it from the
                        // keyboard. A deleting row keeps its progress visible,
                        // and an open menu keeps its anchor.
                        "& .cowboy-session-actions:not(.Mui-disabled):not([data-menu-open='true'])": {
                            opacity: 0,
                            transition: "opacity 120ms ease",
                            "&:hover": { bgcolor: "transparent", color: "text.primary" },
                        },
                        "& [data-desktop-item]:hover .cowboy-session-actions": {
                            opacity: 1,
                            transitionDelay: "250ms",
                        },
                        "& [data-desktop-item]:focus .cowboy-session-actions, & .cowboy-session-actions:focus-visible": {
                            opacity: 1,
                            transitionDelay: "0ms",
                        },
                        // Folder rows: activity and actions share one right-aligned
                        // slot as wide as the two actions. They swap in place
                        // (no reflow); an empty folder keeps the slot for them.
                        "& .cowboy-folder-trailing": {
                            position: "relative",
                            flexShrink: 0,
                            display: "flex",
                            alignItems: "center",
                            justifyContent: "flex-end",
                            minWidth: `calc(2 * ${desktopSize(24)} + 6px)`,
                            ml: 0.5,
                        },
                        "& .cowboy-folder-trailing [data-folder-activity]": {
                            // Desktop rows end flush at the pane splitter; the
                            // filled pill needs its own gap from that edge.
                            mr: desktopSize(6),
                            transition: "opacity 120ms ease",
                        },
                        "& .cowboy-folder-trailing .cowboy-row-actions": {
                            position: "absolute",
                            top: 0,
                            bottom: 0,
                            right: 0,
                            display: "flex",
                            alignItems: "center",
                            gap: "2px",
                            opacity: 0,
                            pointerEvents: "none",
                            transition: "opacity 120ms ease",
                            "& .cowboy-session-actions": { ml: 0 },
                        },
                        "& [data-desktop-item]:hover .cowboy-folder-trailing .cowboy-row-actions": {
                            opacity: 1,
                            pointerEvents: "auto",
                            transitionDelay: "250ms",
                        },
                        "& [data-desktop-item]:hover .cowboy-folder-trailing [data-folder-activity]": {
                            opacity: 0,
                            transitionDelay: "250ms",
                        },
                        "& [data-desktop-item]:focus .cowboy-folder-trailing .cowboy-row-actions, & .cowboy-folder-trailing .cowboy-row-actions:focus-within, & .cowboy-folder-trailing .cowboy-row-actions:has([data-menu-open='true'])": {
                            opacity: 1,
                            pointerEvents: "auto",
                            transitionDelay: "0ms",
                        },
                        "& [data-desktop-item]:focus .cowboy-folder-trailing [data-folder-activity], & .cowboy-folder-trailing:has(.cowboy-row-actions:focus-within) [data-folder-activity], & .cowboy-folder-trailing:has([data-menu-open='true']) [data-folder-activity]": {
                            opacity: 0,
                            transitionDelay: "0ms",
                        },
                    }),
                }}
            >
                {mobileDrawer && <SessionsSyncedCaption />}
                {sortable.order.map((rowKey) => {
                    const row = rowByKey.get(rowKey);
                    if (!row) return null;
                    if (row.kind === "empty") {
                        // The body of an expanded empty folder: keeps the folder
                        // visibly owning its (blank) contents and doubles as the
                        // "into this folder" drop slot. Not a keyboard item.
                        return (
                    <Box
                        component="li"
                        key={rowKey}
                        data-session-folder-empty={row.folder}
                        ref={sortable.registerItem(rowKey)}
                        style={sortable.itemStyle(rowKey)}
                        sx={{
                            ...treeGuideSx(row.depth),
                            ...dropHighlightSx(dropHighlight(row, rowKey)),
                            position: "relative",
                            listStyle: "none",
                            display: "flex",
                            alignItems: "center",
                            minHeight: 36,
                            ...rowIndentSx(row.depth, 18, 12),
                            pr: ROW_END_PR,
                            color: "text.disabled",
                            "@media (pointer: fine) and (hover: hover)": {
                                ...rowIndentSx(row.depth, 18, 12)["@media (pointer: fine) and (hover: hover)"],
                                minHeight: 30,
                            },
                        }}
                    >
                        <Typography variant="caption" sx={{ fontStyle: "italic" }}>
                            {sortable.drag ? "Drop here to file into this folder" : "Empty — drag a session here"}
                        </Typography>
                    </Box>
                        );
                    }
                    if (row.kind === "folder") {
                        const f = row.folder;
                        return (
                    <ReliableListItemButton
                        key={rowKey}
                        data-haptic="selection"
                        data-desktop-item={rowKey}
                        data-desktop-folder-row="true"
                        data-move-pick-dim={movePicking && moveLabelOf(f.id) === undefined ? "true" : undefined}
                        onPointerDown={movePicking && moveLabelOf(f.id) !== undefined
                            ? (event): void => {
                                event.preventDefault();
                                performMove(f.id);
                            }
                            : undefined}
                        data-desktop-pin-active={desktop && pinned ? "true" : undefined}
                        data-drop-target={dropHighlight(row, rowKey) === "header" ? "true" : undefined}
                        aria-expanded={row.expanded}
                        ref={sortable.registerItem(rowKey)}
                        style={sortable.itemStyle(rowKey)}
                        // Collapse chrome is paint-only (icon swap, no transform):
                        // this row lives inside the Mobile swipe compositor.
                        onDragOver={(event) => { if (event.dataTransfer.types.includes(DRAFT_DRAG_TYPE)) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; } }}
                        onDrop={(event) => {
                            const id = event.dataTransfer.getData(DRAFT_DRAG_TYPE);
                            if (!id) return;
                            event.preventDefault(); event.stopPropagation();
                            void draftRepository().document(id).hydrate()
                                .then(() => draftRepository().document(id).change({ type: "move", parent_id: f.id }))
                                .then(() => revealMovedRow(`draft:${id}`, f.id))
                                .catch((error: Error) => documentNotice(error.message));
                        }}
                        onActivate={(): void => setFolderCollapsed([f.id], row.expanded)}
                        sx={{
                            ...(desktop && desktopListItemSx()),
                            ...flushRowSx,
                            ...treeGuideSx(row.depth),
                            ...dropHighlightSx(dropHighlight(row, rowKey)),
                            ...rowIndentSx(row.depth),
                            pr: ROW_END_PR,
                            "@media (pointer: fine) and (hover: hover)": {
                                ...rowIndentSx(row.depth)["@media (pointer: fine) and (hover: hover)"],
                                pr: 0,
                                py: 0.25,
                            },
                        }}
                    >
                        {movePicking && moveLabelOf(f.id) !== undefined && (
                            <MovePickLabel label={moveLabelOf(f.id)!} typed={movePrefix} />
                        )}
                        <Box
                            className="cowboy-session-grip cowboy-folder-chevron"
                            aria-hidden
                            sx={{
                                width: desktopSize(44),
                                height: desktopSize(44),
                                flexShrink: 0,
                                display: "grid",
                                placeItems: "center",
                                color: "text.secondary",
                            }}
                        >
                            {row.expanded
                                ? <ExpandMore sx={{ fontSize: "1.5rem" }} />
                                : <ChevronRight sx={{ fontSize: "1.5rem" }} />}
                        </Box>
                        {/* No leading status dot: one dot cannot say how many
                            agents run, and an empty folder left a hole there.
                            Live counts sit in the trailing activity capsule. */}
                        <ListItemText
                            primary={
                                <Stack
                                    direction="row"
                                    spacing={0.75}
                                    alignItems="center"
                                    sx={{ minWidth: 0 }}
                                >
                                    {row.expanded
                                        ? <FolderOpenOutlined fontSize="medium" sx={{ flexShrink: 0, color: "text.secondary" }} />
                                        : <FolderOutlined fontSize="medium" sx={{ flexShrink: 0, color: "text.secondary" }} />}
                                    <Typography
                                        variant="body2"
                                        noWrap
                                        sx={{ minWidth: 0, fontWeight: 600 }}
                                    >
                                        {f.name}
                                    </Typography>
                                    {f.project && (
                                        <Chip
                                            size="small"
                                            icon={<LabelOutlined sx={{ fontSize: "0.9rem !important" }} />}
                                            label={f.project}
                                            variant="outlined"
                                            sx={{
                                                height: "1.5rem",
                                                maxWidth: "8rem",
                                                fontSize: "0.75rem",
                                                "& .MuiChip-label": {
                                                    px: "0.5rem",
                                                    overflow: "hidden",
                                                    textOverflow: "ellipsis",
                                                },
                                            }}
                                        />
                                    )}
                                    <Typography
                                        variant="caption"
                                        color="text.secondary"
                                        sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}
                                    >
                                        {row.sessionCount}
                                    </Typography>
                                    <SessionAttentionMenu sessions={sessions} folder={f.id} folders={sessionFolders} onPick={onPick} />
                                </Stack>
                            }
                            slotProps={{ primary: { component: "div" } }}
                        />
                        {/* Desktop: one right-aligned slot. Activity rests here and
                            the actions take its place on dwell or keyboard focus,
                            so every folder's badges share one right edge and
                            the hidden actions never leave a gap. Touch keeps
                            its inline layout (`display: contents`). */}
                        <Box className="cowboy-folder-trailing" sx={desktop ? undefined : { display: "contents" }}>
                        <FolderActivityBadges activity={row.activity} total={row.sessionCount} />
                        <Box className="cowboy-row-actions" sx={desktop ? undefined : { display: "contents" }}>
                        {desktop && allowNewSession && onNewInFolder && (
                            <HintTooltip
                                title="New session here"
                                shortcut="A"
                                detail="Uses this directory's defaults"
                                placement="left"
                                enterDelay={600}
                                enterNextDelay={600}
                            >
                                <IconButton
                                    className="cowboy-session-actions"
                                    aria-label={`New session in ${f.name}`}
                                    data-folder-new-session={f.id}
                                    disabled={movePicking}
                                    onClick={(e): void => {
                                        e.stopPropagation();
                                        onNewInFolder(f.id);
                                    }}
                                    sx={{ ml: 0.5, width: desktopSize(44), height: desktopSize(44), flexShrink: 0, position: "relative" }}
                                >
                                    <Add sx={{ fontSize: "1.5rem" }} />
                                </IconButton>
                            </HintTooltip>
                        )}
                        <HintTooltip
                            title={desktop ? "Actions" : ""}
                            detail="M move to · I rename"
                            shortcut="S"
                            placement="left"
                            enterDelay={600}
                            enterNextDelay={600}
                        >
                            <IconButton
                                className="cowboy-session-actions"
                                aria-label={`folder actions ${f.name}`}
                                data-menu-open={folderMenu?.folder.id === f.id ? "true" : undefined}
                                onClick={(e): void => {
                                    e.stopPropagation();
                                    setFolderMenu({ folder: f, el: e.currentTarget });
                                }}
                                sx={{ ml: 0.5, width: desktopSize(44), height: desktopSize(44), flexShrink: 0, position: "relative" }}
                            >
                                <MoreVert sx={{ fontSize: "1.5rem" }} />
                            </IconButton>
                        </HintTooltip>
                        </Box>
                        </Box>
                    </ReliableListItemButton>
                        );
                    }
                    if (row.kind === "draft") return <WorkspaceDraftRow
                        key={`draft:${row.draft.id}`} draft={row.draft}
                        selected={activeId === `draft:${row.draft.id}`} sortable={sortable}
                        desktop={desktop} onPick={() => onPick(`draft:${row.draft.id}`)} onAction={onDraftAction}
                        sx={{ ...(desktop && desktopListItemSx()), ...flushRowSx, ...treeGuideSx(row.depth),
                            ...dropHighlightSx(dropHighlight(row, `draft:${row.draft.id}`)),
                            ...rowIndentSx(row.depth), pr: 0 }}
                    />;
                    const s = row.session;
                    const deleting = deletingSessionIds.has(s.id) || s.closing === true;
                    const jumpLabel = sessionLabelsShown ? jumpLabels.get(s) : undefined;
                    const pulse = fold.pulseSx(s.id);
                    const copyTargeted = copyTarget?.kind === "session" && copyTarget.session.id === s.id;
                    // A `sessions` frame for one busy session must not
                    // re-render every row: a row re-renders only when what it
                    // draws changed. Handlers read the latest callbacks through
                    // `rowLive`; an active drag re-renders every row.
                    return (
                    <SessionRowMemo
                        key={s.id}
                        deps={[
                            s, s.id === activeId, handedOff.has(s.id), deleting, jumpLabel, desktop, pinned, copyTargeted,
                            row.depth, sortable.drag, dropFolderName, dropHighlight(row, s.id),
                            pulse === undefined ? null : (pulse as { animation?: string }).animation,
                            accounts, mobileDrawer,
                        ]}
                        render={() => (
                    <ReliableListItemButton
                        key={s.id}
                        data-haptic="selection"
                        data-desktop-item={s.id}
                        data-desktop-session-row={desktop ? "true" : undefined}
                        data-desktop-current={desktop && s.id === activeId ? "true" : undefined}
                        data-desktop-pin-active={desktop && pinned ? "true" : undefined}
                        data-workspace-copy-target={copyTargeted ? "true" : undefined}
                        data-session-deleting={deleting ? "true" : undefined}
                        aria-busy={deleting || undefined}
                        aria-disabled={deleting || undefined}
                        ref={sortable.registerItem(s.id)}
                        style={sortable.itemStyle(s.id)}
                        selected={s.id === activeId}
                        onDragOver={(event): void => {
                            if (desktop && event.dataTransfer.types.includes(DRAFT_DRAG_TYPE)) {
                                event.preventDefault(); event.dataTransfer.dropEffect = "copy";
                            }
                        }}
                        onDrop={(event): void => {
                            const documentId = event.dataTransfer.getData(DRAFT_DRAG_TYPE);
                            if (!desktop || !documentId) return;
                            event.preventDefault();
                            rowLive.current.onDraftCopy(documentId, s);
                        }}
                        onPointerEnter={desktop
                            ? (event): void => {
                                if (event.pointerType !== "mouse" || s.id === rowLive.current.activeId) return;
                                clearTimeout(hoverPrefetchTimer.current);
                                hoverPrefetchTimer.current = setTimeout(
                                    () => prefetchSessionTail(s.id),
                                    150,
                                );
                            }
                            : undefined}
                        onPointerLeave={desktop
                            ? (): void => clearTimeout(hoverPrefetchTimer.current)
                            : undefined}
                        onActivate={(): void => {
                            if (deleting) return;
                            setPinned(false);
                            rowLive.current.onPick(s.id);
                        }}
                        // The Mobile rail owns its safe-area inset once. Keep
                        // row gutters compact while preserving 44px controls.
                        sx={{
                            ...(desktop && desktopListItemSx()),
                            ...flushRowSx,
                            ...treeGuideSx(row.depth),
                            ...dropHighlightSx(dropHighlight(row, s.id)),
                            "&[data-workspace-copy-target='true']": { bgcolor: "action.selected", outline: "2px solid", outlineColor: "primary.main", outlineOffset: -2 },
                            ...(desktop && s.id === activeId && {
                                // Bind current-session material to the row itself.
                                // This survives both the in-flow rail and the
                                // compact Desktop DetentSheet, whose DOM has no
                                // sessions.list region ancestor. Repeat the
                                // selected/current selectors with higher
                                // specificity so desktopListItemSx's quieter
                                // shared selection material cannot override it.
                                // A flush row is marked by its fill alone: an
                                // edge on one side reads as a lopsided border.
                                "&&.Mui-selected, &&[data-desktop-current='true']": {
                                    bgcolor: (t) => alpha(t.palette.primary.main, CURRENT_ROW_FILL),
                                    boxShadow: "none",
                                },
                                "&&.Mui-selected:hover, &&[data-desktop-current='true']:hover": {
                                    bgcolor: (t) => alpha(t.palette.primary.main, CURRENT_ROW_FILL + 0.04),
                                },
                            }),
                            ...rowIndentSx(row.depth),
                            pr: ROW_END_PR,
                            "@media (pointer: fine) and (hover: hover)": {
                                ...rowIndentSx(row.depth)["@media (pointer: fine) and (hover: hover)"],
                                pr: 0,
                                py: 0.25,
                            },
                            ...(deleting && {
                                pointerEvents: "none",
                                opacity: 0.55,
                            }),
                            // A brief ring so the eye finds the current row
                            // after the fold button reflows the list.
                            ...pulse,
                            // The lifted row is opaque so the rows it passes
                            // over never show through its text.
                            ...(sortable.drag?.id === s.id && {
                                bgcolor: "background.paper",
                                boxShadow: (t: Theme) =>
                                    `0 8px 24px ${alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.5 : 0.18)}`,
                                "&::before": { display: "none" },
                            }),
                        }}
                    >
                        {/* Leading grip — drag to reorder. A real 44px IconButton
                            keeps the Apple HIG touch target stable while the
                            1.25rem glyph follows the global font-size preference.
                            stopPropagation in handleProps keeps a row tap (select)
                            and the sheet's drag separate from a reorder. */}
                        <Box
                            className="cowboy-session-grip-slot"
                            sx={{
                                position: "relative",
                                width: desktopSize(44),
                                height: desktopSize(44),
                                flexShrink: 0,
                                display: "grid",
                                placeItems: "center",
                            }}
                        >
                            {desktop
                                ? (
                                    // The pointer handle names the keyboard
                                    // path; a long dwell keeps an edge sweep
                                    // from opening it.
                                    <HintTooltip
                                        title="Drag to reorder"
                                        shortcut="O"
                                        detail="Order mode · J/K move · Esc done"
                                        placement="right"
                                        enterDelay={600}
                                        enterNextDelay={600}
                                    >
                                        <IconButton
                                            className="cowboy-session-grip"
                                            {...(deleting ? {} : sortable.handleProps(s.id))}
                                            disabled={deleting}
                                            aria-label="Drag to reorder"
                                            sx={{ width: desktopSize(44), height: desktopSize(44), color: "text.disabled" }}
                                        >
                                            <DragIndicator sx={{ fontSize: "1.25rem" }} />
                                        </IconButton>
                                    </HintTooltip>
                                )
                                : (
                                    <IconButton
                                        className="cowboy-session-grip"
                                        {...(deleting ? {} : sortable.handleProps(s.id))}
                                        disabled={deleting}
                                        aria-label="Drag to reorder"
                                        sx={{ width: desktopSize(44), height: desktopSize(44), color: "text.disabled" }}
                                    >
                                        <DragIndicator sx={{ fontSize: "1.25rem" }} />
                                    </IconButton>
                                )}
                        </Box>
                        <ListItemText
                            sx={{ my: 0.25, minWidth: 0 }}
                            primary={<SessionRowTitle session={s} active={s.id === activeId} />}
                            secondary={
                                // Placement belongs with the project it runs: on
                                // the title line the badge took a third of a
                                // phone-width row and truncated the title. The
                                // pending-send badge shares this line: on its own
                                // line it grew and shrank the row around every
                                // send, reflowing the whole list.
                                <Stack
                                    direction="row"
                                    alignItems="center"
                                    sx={{ minWidth: 0, maxWidth: "100%", flexWrap: "wrap", columnGap: 0.5, rowGap: 0.25 }}
                                >
                                    <Typography variant="caption" noWrap sx={{ minWidth: 0 }}>
                                        {sessionListProjectLabel(s)}
                                    </Typography>
                                    <SessionMachineBadge
                                        session={s}
                                        onInfo={() => rowLive.current.onRequestInfo(s)}
                                        compact
                                    />
                                    <AccountBadge account={sessionAccountIdentity(accounts, s)} />
                                    <SessionActionBadge session={s} />
                                    <SessionNextMessageBadge sessionId={s.id} />
                                    {handedOff.has(s.id) && (
                                        // The pair shares a title; this marks the one the work left.
                                        <Chip
                                            data-session-history
                                            size="small"
                                            label="History"
                                            title="Continued in a new agent; this session is read-only"
                                            sx={{
                                                height: "1.25rem", flexShrink: 0, fontSize: "0.6875rem",
                                                border: "1px solid transparent", boxShadow: "none", color: "text.secondary",
                                                bgcolor: (theme) => alpha(theme.palette.text.secondary, theme.palette.mode === "dark" ? 0.22 : 0.14),
                                                "& .MuiChip-label": { px: "0.5rem" },
                                            }}
                                        />
                                    )}
                                    {s.forked_from && (
                                        <Chip
                                            data-session-forked
                                            size="small"
                                            variant="outlined"
                                            label="Fork"
                                            title="Forks an earlier session's conversation with the same agent"
                                            sx={{ height: "1.65em", fontSize: "0.6875rem", flexShrink: 0, "& .MuiChip-label": { px: "0.5em" } }}
                                        />
                                    )}
                                    <SessionUpdateBadge session={s} />
                                    <SessionObligationBadge sessionId={s.id} />
                                    <SessionProjectionBadge sessionId={s.id} />
                                    {(s.system || (s.origin ?? "api") !== "web") && (
                                        <Chip
                                            size="small"
                                            label={s.system ? "System" : "External"}
                                            color={s.system ? "secondary" : "default"}
                                            sx={{ height: "1.65em", fontSize: "0.6875rem", flexShrink: 0, "& .MuiChip-label": { px: "0.5em" } }}
                                        />
                                    )}
                                    <ScheduleBadge meta={s} />
                                </Stack>
                            }
                            slotProps={{
                                primary: { component: "div" },
                                secondary: {
                                    component: "div",
                                    noWrap: true,
                                    variant: "caption",
                                },
                            }}
                        />
                        {sortable.drag?.id === s.id && (
                            // Name the destination while dragging, so the drop
                            // never has to be guessed from the gap alone.
                            <Chip
                                size="small"
                                color="primary"
                                icon={dropFolderName
                                    ? <FolderOutlined sx={{ fontSize: "0.95rem !important" }} />
                                    : undefined}
                                label={dropFolderName ?? "Global · top level"}
                                data-session-drop-destination
                                sx={{
                                    // A tag on the lifted row's top edge: it
                                    // never squeezes the title it travels with.
                                    position: "absolute",
                                    top: -10,
                                    right: 48,
                                    zIndex: 1,
                                    boxShadow: 2,
                                    pointerEvents: "none",
                                    maxWidth: "9rem",
                                    height: "1.5rem",
                                    fontSize: "0.75rem",
                                    fontWeight: 650,
                                    "& .MuiChip-label": { overflow: "hidden", textOverflow: "ellipsis" },
                                }}
                            />
                        )}
                        {jumpLabel && (
                            <Suspense fallback={null}>
                                <DesktopSessionShortcut label={jumpLabel} />
                            </Suspense>
                        )}
                        <HintTooltip
                            title={desktop && !deleting ? "Actions" : ""}
                            detail="M move to · I rename"
                            shortcut="S"
                            placement="left"
                            enterDelay={600}
                            enterNextDelay={600}
                        >
                            <IconButton
                                className="cowboy-session-actions"
                                aria-label={deleting ? `Deleting ${s.title}` : `row actions ${s.id}`}
                                disabled={deleting}
                                data-menu-open={menuAnchor?.row.id === s.id ? "true" : undefined}
                                onClick={(e): void => {
                                    e.stopPropagation();
                                    if (deleting) return;
                                    setMenuAnchor({ row: s, el: e.currentTarget });
                                }}
                                // A 44px Apple-HIG tap target on touch; on Desktop the
                                // target and its 1.5rem glyph track the global font size.
                                sx={{ ml: 0.5, width: desktopSize(44), height: desktopSize(44), flexShrink: 0, position: "relative" }}
                            >
                                {deleting
                                    ? <DelayedNetworkProgress size={desktopSize(18)} />
                                    : <MoreVert sx={{ fontSize: "1.5rem" }} />}
                            </IconButton>
                        </HintTooltip>
                    </ReliableListItemButton>
                        )}
                    />
                    );
                })}
                {loaded && !movePicking && (
                    <SessionTrashSection
                        tree={trashTree}
                        expanded={trashExpanded}
                        onToggle={(key): void => toggleTrash(key)}
                        retentionDays={trashRetention.days}
                        now={trashNow}
                        desktop={desktop}
                        rowSx={trashRowSx}
                        onOpenRow={setTrashRow}
                        onOpenMenu={(): void => setTrashMenu(true)}
                    />
                )}
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
            {desktop && (
                // The cursor row's own keys, beside the list they act on. The
                // window status line lists them too, but far from the rail;
                // shown only while Sessions owns focus (FOCUS.md "Sessions").
                <Box
                    data-sessions-row-keys
                    aria-hidden
                    sx={{
                        display: "none",
                        "[data-desktop-region='sessions.list'][data-desktop-focused='true'] &": {
                            display: "flex",
                        },
                        flexShrink: 0,
                        flexWrap: "wrap",
                        alignItems: "center",
                        columnGap: 1,
                        rowGap: 0.5,
                        px: 1,
                        py: 0.75,
                        borderTop: 1,
                        borderColor: "divider",
                        typography: "caption",
                        color: "text.secondary",
                        "& > span": { display: "inline-flex", alignItems: "center", whiteSpace: "nowrap" },
                        "& .MuiBox-root, & kbd": { ml: "0 !important", mr: 0.5 },
                    }}
                >
                    {([
                        ["Enter", "Open"],
                        ["R", "Rename"],
                        ["M", "Move"],
                        ["X", "Trash"],
                        [",", "Settings"],
                        ["A", "New here"],
                        ["S", "More"],
                    ] as const).map(([key, label]) => (
                        <span key={key}><Kbd keys={key} />{label}</span>
                    ))}
                </Box>
            )}
            {mobileDrawer && (
                <MobileSessionsDock
                    phone={phone}
                    fold={fold}
                    onNew={allowNewSession ? onNew : undefined}
                    onClose={onClose}
                    onOpenAbout={onOpenAbout}
                    onOpenSettings={onOpenSettings}
                />
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
                    onClick={requestRenameFromMenu}
                >
                    <ListItemIcon>
                        <DriveFileRenameOutline fontSize="medium" />
                    </ListItemIcon>
                    <ListItemText primary="Rename" />
                </MenuItem>
                <MenuItem
                    onClick={(): void => {
                        if (menuAnchor) setMovePicker({ kind: "session", id: menuAnchor.row.id });
                        setMenuAnchor(null);
                    }}
                >
                    <ListItemIcon>
                        <DriveFileMoveOutlined fontSize="medium" />
                    </ListItemIcon>
                    <ListItemText primary="Move to folder…" />
                </MenuItem>
                {menuAnchor && !menuHandedOff && !menuAnchor.row.system && !isManagedChild(menuAnchor.row) && (
                    <MenuItem
                        data-session-new-from
                        onClick={(): void => {
                            const row = menuAnchor.row;
                            setMenuAnchor(null);
                            requestPickSession(row.id, row);
                            requestNewFrom(row.id);
                        }}
                    >
                        <ListItemIcon>
                            <ForkRightOutlined fontSize="medium" />
                        </ListItemIcon>
                        <ListItemText primary="Replace or fork" />
                    </MenuItem>
                )}
                {menuAnchor && tree.folderOf.get(menuAnchor.row.id) && (
                    <MenuItem onClick={(): void => {
                        moveItem({ kind: "session", id: menuAnchor.row.id }, null);
                        setMenuAnchor(null);
                    }}>
                        <ListItemIcon><DriveFileMoveOutlined fontSize="medium" /></ListItemIcon>
                        <ListItemText primary="Move to Global" secondary="Outside all folders" />
                    </MenuItem>
                )}
                <MenuItem onClick={toggleMenuSessionNotifications}>
                    <ListItemIcon>
                        {menuSessionMuted
                            ? <NotificationsOutlined fontSize="medium" />
                            : <NotificationsOffOutlined fontSize="medium" />}
                    </ListItemIcon>
                    <ListItemText primary={menuSessionMuted ? "Unmute notifications" : "Mute notifications"} />
                </MenuItem>
                {menuHibernate && (
                    <MenuItem disabled={menuHibernate === "busy"} onClick={hibernateMenuSession}>
                        <ListItemIcon>
                            <BedtimeOutlined fontSize="medium" />
                        </ListItemIcon>
                        <ListItemText
                            primary="Hibernate"
                            secondary={menuHibernate === "busy" ? "Available after the current turn" : "Free its memory; resumes when opened"}
                        />
                    </MenuItem>
                )}
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
                    <ListItemText primary="Move to Trash" />
                </MenuItem>
            </Menu>
            {desktop && (
                <DesktopModalShell
                        open={!!menuAnchor}
                        onClose={(): void => setMenuAnchor(null)}
                        title="Session"
                        description={`${menuAnchor?.row.title ?? ""} · ${menuAnchor?.row.cwd ?? ""}`}
                        width={920}
                        onShortcutKeyDown={(event): void => {
                            if (
                                desktopImeOwnsKey(event.nativeEvent) ||
                                event.metaKey || event.ctrlKey || event.altKey || event.shiftKey
                            ) return;
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
                            sx={{ p: 2 }}
                        >
                            <Box sx={{ display: "grid", gridTemplateColumns: "minmax(0, 1.2fr) minmax(280px, 0.8fr)", gap: 2.5 }}>
                        <DesktopSessionInfoPanel session={menuAnchor?.row ?? null} />
                        <Stack
                            data-desktop-session-actions
                            spacing={0.5}
                        >
                            <Typography variant="overline" color="text.secondary" sx={{ px: 1 }}>Actions</Typography>
                            <Button data-session-shortcut="," autoFocus fullWidth startIcon={<SettingsIcon />} onClick={(): void => { if (!menuAnchor) return; const row = menuAnchor.row; setMenuAnchor(null); requestPickSession(row.id, row); openSessionSettings("agent", row.id); }} sx={{ justifyContent: "flex-start" }}>
                                <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Settings…</Box>
                                <Kbd keys="," />
                            </Button>
                            <Button data-session-shortcut="r" fullWidth startIcon={<DriveFileRenameOutline />} onClick={requestRenameFromMenu} sx={{ justifyContent: "flex-start" }}>
                                <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Rename</Box>
                                <Kbd keys="R" />
                            </Button>
                            <Button data-session-shortcut="m" fullWidth startIcon={<DriveFileMoveOutlined />} onClick={(): void => { if (menuAnchor) setMovePicker({ kind: "session", id: menuAnchor.row.id }); setMenuAnchor(null); }} sx={{ justifyContent: "flex-start" }}>
                                <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Move to folder…</Box>
                                <Kbd keys="M" />
                            </Button>
                            {menuAnchor && !menuHandedOff && !menuAnchor.row.system && !isManagedChild(menuAnchor.row) && (
                                <Button data-session-shortcut="f" data-session-new-from fullWidth startIcon={<ForkRightOutlined />} onClick={(): void => { const row = menuAnchor.row; setMenuAnchor(null); requestPickSession(row.id, row); requestNewFrom(row.id); }} sx={{ justifyContent: "flex-start" }}>
                                    <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Replace or fork</Box>
                                    <Kbd keys="F" />
                                </Button>
                            )}
                            <Button data-session-shortcut="l" fullWidth startIcon={<RefreshIcon />} onClick={(): void => { if (menuAnchor) onRequestReload(menuAnchor.row); setMenuAnchor(null); }} sx={{ justifyContent: "flex-start" }}>
                                <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Reload</Box>
                                <Kbd keys="L" />
                            </Button>
                            <Button data-session-shortcut="n" fullWidth startIcon={menuSessionMuted ? <NotificationsOutlined /> : <NotificationsOffOutlined />} onClick={toggleMenuSessionNotifications} sx={{ justifyContent: "flex-start" }}>
                                <Box component="span" sx={{ flex: 1, textAlign: "left" }}>{menuSessionMuted ? "Unmute notifications" : "Mute notifications"}</Box>
                                <Kbd keys="N" />
                            </Button>
                            {menuHibernate && (
                                <Button data-session-shortcut="z" fullWidth disabled={menuHibernate === "busy"} startIcon={<BedtimeOutlined />} onClick={hibernateMenuSession} sx={{ justifyContent: "flex-start" }}>
                                    <Box component="span" sx={{ flex: 1, textAlign: "left" }}>{menuHibernate === "busy" ? "Hibernate after this turn" : "Hibernate"}</Box>
                                    <Kbd keys="Z" />
                                </Button>
                            )}
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
                            <Button data-session-shortcut="x" color="error" fullWidth startIcon={<DeleteOutline />} onClick={(): void => { if (menuAnchor) onRequestDelete(menuAnchor.row); setMenuAnchor(null); }} sx={{ justifyContent: "flex-start" }}>
                                <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Move to Trash</Box>
                                <Kbd keys="X" />
                            </Button>
                        </Stack>
                            </Box>
                        </Box>
                </DesktopModalShell>
            )}
            {/* --- Folder layers (docs/sessions-folders.md) --- */}
            <Menu
                sx={{ display: desktop ? "none" : undefined }}
                anchorEl={folderMenu?.el ?? null}
                open={!desktop && !!folderMenu}
                onClose={(): void => setFolderMenu(null)}
                slotProps={{ paper: { sx: { minWidth: 220 } } }}
            >
                {allowNewSession && onNewInFolder && (
                    <MenuItem
                        onClick={(): void => {
                            if (folderMenuFolder) onNewInFolder(folderMenuFolder.id);
                            setFolderMenu(null);
                        }}
                    >
                        <ListItemIcon><Add fontSize="medium" /></ListItemIcon>
                        <ListItemText primary="New session here" />
                    </MenuItem>
                )}
                <MenuItem onClick={(): void => { if (folderMenuFolder) setDefaultsFolder(folderMenuFolder.id); setFolderMenu(null); }}>
                    <ListItemIcon><SettingsIcon fontSize="medium" /></ListItemIcon>
                    <ListItemText primary="Directory defaults…" />
                </MenuItem>
                <MenuItem onClick={(): void => { if (folderMenuFolder) setSelectFolderSessions(folderMenuFolder.id); setFolderMenu(null); }}>
                    <ListItemIcon><ChecklistOutlined fontSize="medium" /></ListItemIcon>
                    <ListItemText primary="Select sessions…" secondary="Delete several sessions" />
                </MenuItem>
                <MenuItem
                    onClick={(): void => {
                        if (folderMenuFolder) openFolderName({ mode: "create", parent: folderMenuFolder.id });
                    }}
                >
                    <ListItemIcon><CreateNewFolderOutlined fontSize="medium" /></ListItemIcon>
                    <ListItemText primary="New folder inside" />
                </MenuItem>
                <MenuItem
                    onClick={(): void => {
                        if (folderMenuFolder) openFolderName({ mode: "rename", folder: folderMenuFolder });
                    }}
                >
                    <ListItemIcon><DriveFileRenameOutline fontSize="medium" /></ListItemIcon>
                    <ListItemText primary="Rename" />
                </MenuItem>
                <MenuItem
                    onClick={(): void => {
                        if (folderMenuFolder) setMovePicker({ kind: "folder", id: folderMenuFolder.id });
                        setFolderMenu(null);
                    }}
                >
                    <ListItemIcon><DriveFileMoveOutlined fontSize="medium" /></ListItemIcon>
                    <ListItemText primary="Move to…" />
                </MenuItem>
                {folderMenuFolder?.parent && (
                    <MenuItem onClick={(): void => {
                        moveItem({ kind: "folder", id: folderMenuFolder.id }, null);
                        setFolderMenu(null);
                    }}>
                        <ListItemIcon><DriveFileMoveOutlined fontSize="medium" /></ListItemIcon>
                        <ListItemText primary="Move to Global" secondary="Outside all folders" />
                    </MenuItem>
                )}
                <MenuItem
                    onClick={(): void => {
                        setProjectPicker(folderMenuFolder);
                        setFolderMenu(null);
                    }}
                >
                    <ListItemIcon><LabelOutlined fontSize="medium" /></ListItemIcon>
                    <ListItemText
                        primary="Auto-file project sessions…"
                        secondary={folderMenuFolder?.project ?? undefined}
                    />
                </MenuItem>
                <Divider />
                {unboundProjects.length > 0 && <MenuItem onClick={(): void => { organizeSessionsByProject(); setFolderMenu(null); }}>
                    <ListItemIcon><LabelOutlined fontSize="medium" /></ListItemIcon>
                    <ListItemText primary="Organize all projects" secondary={`Create or bind ${unboundProjects.length} project folders`} />
                </MenuItem>}
                <MenuItem
                    onClick={(): void => {
                        setDeleteFolder(folderMenuFolder);
                        setFolderMenu(null);
                    }}
                    sx={{ color: "error.main" }}
                >
                    <ListItemIcon><DeleteOutline fontSize="medium" color="error" /></ListItemIcon>
                    <ListItemText primary="Delete folder" />
                </MenuItem>
            </Menu>
            {desktop && (
                <DesktopModalShell
                    open={!!folderMenu}
                    onClose={(): void => setFolderMenu(null)}
                    title="Folder"
                    description={`${folderMenuFolder?.name ?? ""}${folderMenuFolder?.project && folderMenuFolder.project !== folderMenuFolder.name ? ` · project ${folderMenuFolder.project}` : ""} · ${String(folderMenuCount?.kind === "folder" ? folderMenuCount.sessionCount : 0)} items`}
                    width={560}
                    onShortcutKeyDown={(event): void => {
                        if (
                            desktopImeOwnsKey(event.nativeEvent) ||
                            event.metaKey || event.ctrlKey || event.altKey || event.shiftKey
                        ) return;
                        const key = workspaceCommandKey(event.nativeEvent).toLowerCase();
                        const directAction = event.currentTarget.querySelector<HTMLButtonElement>(
                            `[data-folder-shortcut="${key}"], [data-folder-shortcut-alt="${key}"]`,
                        );
                        if (directAction) {
                            event.preventDefault();
                            event.stopPropagation();
                            directAction.click();
                            return;
                        }
                        if (key !== "j" && key !== "k") return;
                        const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>(
                            "[data-desktop-folder-actions] button:not(:disabled)",
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
                    <Stack data-desktop-folder-actions spacing={0.5} sx={{ p: 2 }}>
                        <Typography variant="overline" color="text.secondary" sx={{ px: 1 }}>Actions</Typography>
                        {allowNewSession && onNewInFolder && (
                            <Button data-folder-shortcut="n" autoFocus fullWidth startIcon={<Add />} onClick={(): void => { if (folderMenuFolder) onNewInFolder(folderMenuFolder.id); setFolderMenu(null); }} sx={{ justifyContent: "flex-start" }}>
                                <Box component="span" sx={{ flex: 1, textAlign: "left" }}>New session here</Box>
                                <Kbd keys="N" />
                            </Button>
                        )}
                        <Button data-folder-shortcut="f" fullWidth startIcon={<CreateNewFolderOutlined />} onClick={(): void => { if (folderMenuFolder) openFolderName({ mode: "create", parent: folderMenuFolder.id }); }} sx={{ justifyContent: "flex-start" }}>
                            <Box component="span" sx={{ flex: 1, textAlign: "left" }}>New folder inside</Box>
                            <Kbd keys="F" />
                        </Button>
                        <Button data-folder-shortcut="r" fullWidth startIcon={<DriveFileRenameOutline />} onClick={(): void => { if (folderMenuFolder) openFolderName({ mode: "rename", folder: folderMenuFolder }); }} sx={{ justifyContent: "flex-start" }}>
                            <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Rename</Box>
                            <Kbd keys="R" />
                        </Button>
                        <Button data-folder-shortcut="m" fullWidth startIcon={<DriveFileMoveOutlined />} onClick={(): void => { if (folderMenuFolder) setMovePicker({ kind: "folder", id: folderMenuFolder.id }); setFolderMenu(null); }} sx={{ justifyContent: "flex-start" }}>
                            <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Move to…</Box>
                            <Kbd keys="M" />
                        </Button>
                        <Button data-folder-shortcut="d" data-folder-shortcut-alt="," fullWidth startIcon={<SettingsIcon />} onClick={(): void => { if (folderMenuFolder) setDefaultsFolder(folderMenuFolder.id); setFolderMenu(null); }} sx={{ justifyContent: "flex-start" }}>
                            <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Directory defaults…</Box><Kbd keys="D" />
                        </Button>
                        <Button data-folder-shortcut="b" fullWidth startIcon={<LabelOutlined />} onClick={(): void => { setProjectPicker(folderMenuFolder); setFolderMenu(null); }} sx={{ justifyContent: "flex-start" }}>
                            <Box component="span" sx={{ flex: 1, textAlign: "left" }}>{folderMenuFolder?.project ? `Auto-file: ${folderMenuFolder.project}` : "Auto-file project sessions…"}</Box>
                            <Kbd keys="B" />
                        </Button>
                        <Button data-folder-shortcut="s" fullWidth startIcon={<ChecklistOutlined />} onClick={(): void => { if (folderMenuFolder) setSelectFolderSessions(folderMenuFolder.id); setFolderMenu(null); }} sx={{ justifyContent: "flex-start" }}>
                            <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Select sessions…</Box><Kbd keys="S" />
                        </Button>
                        <Divider sx={{ my: 0.5 }} />
                        {unboundProjects.length > 0 && <Button data-folder-shortcut="p" fullWidth startIcon={<LabelOutlined />} onClick={(): void => { organizeSessionsByProject(); setFolderMenu(null); }} sx={{ justifyContent: "flex-start" }}>
                            <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Organize all projects ({unboundProjects.length})</Box><Kbd keys="P" />
                        </Button>}
                        <Button data-folder-shortcut="x" color="error" fullWidth startIcon={<DeleteOutline />} onClick={(): void => { setDeleteFolder(folderMenuFolder); setFolderMenu(null); }} sx={{ justifyContent: "flex-start" }}>
                            <Box component="span" sx={{ flex: 1, textAlign: "left" }}>Delete folder</Box>
                            <Kbd keys="X" />
                        </Button>
                    </Stack>
                </DesktopModalShell>
            )}
            {namePrompt?.mode === "create" && (
                <InAppRoot>
                    <CreateDialog
                        open
                        initialVariant="folder"
                        folderOnly
                        initialFolder={namePrompt.parent}
                        onClose={(): void => setNamePrompt(null)}
                        onCreated={(): void => {}}
                        onFolderCreated={(id, parent): void => {
                            if (parent) setFolderCollapsed([parent], false);
                            if (desktop) focusRow(`folder:${id}`);
                        }}
                    />
                </InAppRoot>
            )}
            {namePrompt?.mode === "rename" && (
                <FolderNameShell
                    title="Rename folder"
                    initial={namePrompt.folder.name}
                    confirmLabel="Save"
                    onClose={(): void => setNamePrompt(null)}
                    onConfirm={(name): void => {
                        renameSessionFolder(namePrompt.folder.id, name);
                        setNamePrompt(null);
                    }}
                />
            )}
            {movePicker && (
                <FolderPickerShell
                    title={movePicker.kind === "folder"
                        ? `Move "${sessionFolderById(sessionFolders, movePicker.id)?.name ?? ""}" to`
                        : `Move "${byId.get(movePicker.id)?.title ?? ""}" to`}
                    value={sessionFolders}
                    current={movePicker.kind === "folder"
                        ? sessionFolderById(sessionFolders, movePicker.id)?.parent ?? null
                        : tree.folderOf.get(movePicker.id) ?? null}
                    exclude={movePicker.kind === "folder" ? movePicker.id : null}
                    onPick={(folder): void => {
                        moveItem(movePicker, folder);
                        setMovePicker(null);
                    }}
                    onNewFolder={(): void =>
                        openFolderName({
                            mode: "create",
                            parent: movePicker.kind === "folder"
                                ? sessionFolderById(sessionFolders, movePicker.id)?.parent ?? null
                                : tree.folderOf.get(movePicker.id) ?? null,
                        })}
                    onClose={(): void => setMovePicker(null)}
                />
            )}
            {defaultsFolder && <FolderDefaultsSheet id={defaultsFolder} onClose={(): void => setDefaultsFolder(null)} />}
            {selectFolderSessions && <DirectorySessionsSheet id={selectFolderSessions} onClose={(): void => setSelectFolderSessions(null)} />}
            {projectPicker && (
                <ProjectPickerShell
                    folder={projectPicker}
                    labels={projectLabels}
                    onPick={(project): void => {
                        bindSessionFolderProject(projectPicker.id, project);
                        setProjectPicker(null);
                    }}
                    onClose={(): void => setProjectPicker(null)}
                />
            )}
            {deleteFolder && deleteFolderPlan && (
                <DeleteFolderToTrashShell
                    folder={deleteFolder}
                    plan={deleteFolderPlan}
                    destination={deleteFolder.parent}
                    destinationName={sessionFolderLocation(sessionFolders, deleteFolder.parent)}
                    retentionDays={trashRetention.days}
                    onClose={(): void => setDeleteFolder(null)}
                    onDeleteFolderOnly={(): void => {
                        removeSessionFolder(deleteFolder.id);
                        setDeleteFolder(null);
                    }}
                />
            )}
            {trashRow && (
                <TrashItemSheet
                    row={trashRow}
                    retentionDays={trashRetention.days}
                    now={Date.now()}
                    onClose={(): void => setTrashRow(null)}
                    onOpenSession={(id): void => {
                        revealSession(id);
                        onPick(id);
                    }}
                />
            )}
            {trashMenu && (
                <TrashMenuSheet
                    tree={trashTree}
                    retention={trashRetention}
                    onClose={(): void => setTrashMenu(false)}
                />
            )}
        </Stack>
    );
}

export type WorkspaceWorkItem = {
    id: string;
    title: string;
    projects: string[];
    recipe: string;
    blocked: boolean;
};

export type WorkspaceChoice = {
    value: string;
    label: string;
    help: string;
    project?: string;
    active_work_items: readonly WorkspaceWorkItem[];
};

export const NO_SESSIONS: readonly SessionMeta[] = [];

export function CreateDialog({
    open,
    onClose,
    onCreated,
    initialFolder = null,
    initialVariant = "session",
    folderOnly = false,
    onFolderCreated,
    onDraftCreated,
}: {
    open: boolean;
    onClose: () => void;
    /** Called with a local projection so the UI can focus it before the WS list catches up. */
    onCreated: (session: SessionMeta, folder: string | null) => void;
    initialFolder?: string | null;
    initialVariant?: CreateVariant;
    /** Folder-tree shortcuts keep one focused task; Create still offers all types. */
    folderOnly?: boolean;
    onFolderCreated?: (id: string, parent: string | null) => void;
    onDraftCreated?: (id: string) => void;
}): React.JSX.Element {
    const keyboardOpen = useKeyboardOpen();
    const [variant, setVariant] = useState<CreateVariant>(folderOnly ? "folder" : initialVariant);
    const [draftTitle, setDraftTitle] = useState("");
    const selectDraftTitleOnFocus = useRef(true);
    const creatingRef = useRef(false);
    // One key per opened sheet: a repeated submit of the same Create names the
    // same session to the daemon instead of creating a second one.
    const createKeyRef = useRef("");
    const machines = useStoreSelector((snapshot) => snapshot.machines);
    const sessionDirectories = useStoreSelector((snapshot) => snapshot.sessionFolders);
    const [directory, setDirectory] = useState(initialFolder ?? "");
    const directoryDefaults = resolveFolderDefaults(sessionDirectories, directory || null);
    // The sheet stays mounted: a closed one must not re-render on session frames.
    const allSessions = useStoreSelector((snapshot) => open ? snapshot.sessions : NO_SESSIONS);
    // Fields the directory leaves unset follow its most recent session, so a
    // directory used for one project needs no configuration to stay consistent.
    const sessionHints = useMemo(
        () => folderSessionHints(allSessions, sessionDirectories, directory || null),
        [allSessions, sessionDirectories, directory],
    );
    const placement = useProjectPlacement(open && !folderOnly, machines, directoryDefaults.values, sessionHints);
    const [presetOverride, setPresetOverride] = useState<string | undefined>();
    const fullInstallation = placement.installations.find((installation) => installation.full);
    const machineId = placement.machineId;
    const cwd = placement.project?.projectId ?? "";
    const provider = placement.installation?.provider ?? "";
    const subscription = useSessionSubscription(
        open && variant === "session",
        provider,
        sessionHints?.provider === provider ? sessionHints.account : undefined,
    );
    const desktop = useSurfaceProfile().kind === "desktop";
    // Desktop keyboard layer (FOCUS.md "Modals"): a text field is Insert,
    // everything else is Normal, where the shared modal grammar runs (digits
    // and H/L pick the type, J/K move, I edits). Drives the visible slots.
    const [focusZone, setFocusZone] = useState<"tabs" | "text" | "other">("text");
    // Session and Folder share one Sessions-tree location: both place an
    // item in the same tree, so switching between them keeps the choice.
    const directoryChoices = useMemo(() => sessionDirectoryChoices(sessionDirectories, true), [sessionDirectories]);
    const [folderName, setFolderName] = useState("");
    const [folderDefaults, setFolderDefaults] = useState<FolderDefaults>({});
    const [folderDefaultsExpanded, setFolderDefaultsExpanded] = useState(false);
    const [folderDefaultField, setFolderDefaultField] = useState<FolderDefaultKey | null>(null);
    const editingFolderDefault = variant === "folder" && folderDefaultField !== null;
    const folderEffective = { ...directoryDefaults.values, ...folderDefaults };
    const folderPresetMismatch = Boolean(folderEffective.preset && !folderEffective.preset.startsWith(`${folderEffective.provider ?? "claude-code"}/`));
    const { catalog: folderCatalog, error: folderCatalogError } = useProviderCatalog(open && variant === "folder");
    // Cancel, Esc, backdrop and swipe-down ask before throwing away input.
    // Captured at open, so the generated default titles do not count as edits.
    const openedDefaults = useRef({ title: "", draftTitle: "" });
    const [confirmDiscard, setConfirmDiscard] = useState(false);
    const requestClose = (): void => {
        if (!formDirty()) { onClose(); return; }
        // Open after the initiating Escape finishes bubbling, or the new
        // confirmation's own escape listener would close it at once.
        globalThis.requestAnimationFrame(() => setConfirmDiscard(true));
    };
    const discardAndClose = (): void => { setConfirmDiscard(false); onClose(); };
    const closeLevel = (): void => { if (editingFolderDefault) setFolderDefaultField(null); else requestClose(); };
    const [workItemId, setWorkItemId] = useState("");
    const [contextSources, setContextSources] = useState<string[]>([]);
    const [creating, setCreating] = useState(false);
    const [createError, setCreateError] = useState("");
    const selectedWorkspace: WorkspaceChoice | undefined = placement.project ? {
        value: placement.project.projectId,
        label: placement.project.name,
        help: machines.find((m) => m.id === machineId)?.workspaces.find((w) => w.id === cwd)?.canonical_path ?? "",
        active_work_items: [],
    } : undefined;
    const selectedProviderEntry = placement.installation?.entry;
    const selectedPreset = newSessionPreset(provider, selectedProviderEntry?.manifest.configuration.presets ?? [], directoryDefaults.values.preset, presetOverride);
    const selectedInstalledProvider = placement.installation?.installed;
    const presetAvailable = !selectedPreset || Boolean(selectedProviderEntry?.manifest.configuration.presets.some((preset) => `${provider}/${preset.id}` === selectedPreset));
    const providerAvailable = (candidate: string): boolean => Boolean(candidate && placement.ready && candidate === provider);
    const selectedWorkItem = selectedWorkspace?.active_work_items.find((item) => item.id === workItemId);
    // Editable session title. Empty on Create → renameSession no-ops → the
    // daemon's default + first-prompt auto-title apply. RESET to a fresh default
    // on every open (below): the sheet stays mounted, so this state would
    // otherwise survive and show the last-typed value.
    const [title, setTitle] = useState<string>("");
    const titleRef = useRef<HTMLInputElement>(null);
    const titleFocusTimer = useRef<ReturnType<typeof globalThis.setTimeout> | undefined>(undefined);
    // Session count, captured in a ref so its default name is computed at open
    // time WITHOUT a session arriving mid-edit clobbering what you're typing.
    const sessionCount = useStoreSelector((snapshot) => snapshot.sessions.length);
    const sessionCountRef = useRef(sessionCount);
    sessionCountRef.current = sessionCount;
    // A fresh Create selects its default title for immediate replacement.
    // Touch transfers the opener's in-gesture keyboard claim after mounting.
    useEffect(() => {
        if (!open) return undefined;
        installNativeCompositionTracker();
        setFocusZone("text");
        setVariant(folderOnly ? "folder" : initialVariant);
        const initialDraftTitle = defaultDraftTitle();
        const initialTitle = `New session ${sessionCountRef.current + 1}`;
        openedDefaults.current = { title: initialTitle, draftTitle: initialDraftTitle };
        setConfirmDiscard(false);
        setDraftTitle(initialDraftTitle);
        selectDraftTitleOnFocus.current = true;
        creatingRef.current = false;
        createKeyRef.current = newUuid();
        setTitle(initialTitle);
        setDirectory(initialFolder ?? "");
        setPresetOverride(undefined);
        setFolderName("");
        setFolderDefaults({});
        setFolderDefaultsExpanded(false);
        setFolderDefaultField(null);
        setWorkItemId("");
        setContextSources([]);
        setCreating(false);
        setCreateError("");
        const t = globalThis.setTimeout(() => {
            titleRef.current?.focus({ preventScroll: true });
            titleRef.current?.select();
        }, 120);
        titleFocusTimer.current = t;
        return () => {
            globalThis.clearTimeout(t);
            if (titleFocusTimer.current === t) titleFocusTimer.current = undefined;
        };
    }, [open]);
    function formDirty(): boolean {
        return folderName.trim() !== "" || Object.keys(folderDefaults).length > 0 ||
            title !== openedDefaults.current.title || draftTitle !== openedDefaults.current.draftTitle ||
            directory !== (initialFolder ?? "") || presetOverride !== undefined ||
            workItemId !== "" || contextSources.length > 0 ||
            placement.chosen || subscription.chosen;
    }
    const navbarAtBottom = useNavbarAtBottom();
    // Field geometry for the touch Create cover, whose sheet a phone has
    // shown stranded part-way down above the keyboard (sheetKeyboardDiagnostics).
    useSheetKeyboardDiagnostics(
        "create",
        open && !desktop,
        () => titleRef.current?.closest("[role='dialog']") ?? null,
    );
    const theme = useTheme();
    const create = (): void => {
        if (creatingRef.current) return;
        if (editingFolderDefault) { setFolderDefaultField(null); return; }
        if (variant === "folder") {
            if (folderPresetMismatch) return;
            const name = normalizeSessionFolderName(folderName);
            if (name === null) return;
            if (directory && !sessionDirectories.folders.some((folder) => folder.id === directory)) {
                setCreateError("The Sessions directory was removed. Choose another directory or Global.");
                return;
            }
            // Folders are synced presentation state: the create is optimistic
            // and local, so the sheet closes at once and the Sessions list
            // reveals the new row.
            creatingRef.current = true;
            const id = createSessionFolder(name, directory || null, null, folderDefaults);
            if (id) {
                announceSessionFolderCreated(id, directory || null);
                onFolderCreated?.(id, directory || null);
            }
            onClose();
            return;
        }
        if (variant === "draft") {
            const repository = draftRepository();
            if (directory && !sessionDirectories.folders.some((entry) => entry.id === directory)) {
                setCreateError("The Draft directory was removed. Choose another directory or clear it for the top level.");
                return;
            }
            creatingRef.current = true;
            setCreating(true);
            setCreateError("");
            void repository.create(draftTitle.trim() || defaultDraftTitle(), directory || null)
                .then((id): void => {
                    onClose();
                    if (onDraftCreated) onDraftCreated(id);
                    else openDrafts(id);
                })
                .catch((error: unknown): void => {
                    setCreateError(error instanceof Error ? error.message : "Draft creation failed");
                    creatingRef.current = false;
                    setCreating(false);
                });
            return;
        }
        if (creating || !placement.ready || !providerAvailable(provider) || !presetAvailable || !machineId || !cwd) return;
        if (directory && !sessionDirectories.folders.some((folder) => folder.id === directory)) {
            setCreateError("The Sessions directory was removed. Choose another directory or Global.");
            return;
        }
        if (!subscription.allowed) {
            setCreateError("Choose an available subscription before creating this session.");
            return;
        }
        // POST (not the fire-and-forget WS `new_session`) so we get the assigned
        // id back synchronously and can focus the new session the moment it's
        // created.
        creatingRef.current = true;
        setCreating(true);
        setCreateError("");
        void (async (): Promise<void> => {
            let created = false;
            try {
                const response = await fetch(placement.separate ? "/api/execution-sessions" : "/api/sessions", {
                    method: "POST",
                    headers: {
                        "content-type": "application/json",
                        "idempotency-key": createKeyRef.current,
                    },
                    body: JSON.stringify({
                        provider,
                        provider_account_id: subscription.value || undefined,
                        context_sources: contextSources,
                        machine_id: machineId,
                        runtime_machine_id: placement.runtimeMachineId,
                        cwd,
                        folder_id: directory,
                        preset: selectedPreset,
                        origin: "web",
                        initial_prompt: selectedWorkItem
                            ? `Resume Columbus work item ${selectedWorkItem.id}. Read its durable metadata and relevant artifact, then set the native agent goal when supported. Keep plan, progress, review, and session state in the active agent runtime.`
                            : undefined,
                    }),
                });
                if (!response.ok) {
                    const detail = (await response.text()).trim();
                    throw new Error(detail || `Session creation failed (${response.status})`);
                }
                const data = await response.json() as {
                    session_id?: string;
                    provider_version?: string;
                    provider_generation_digest?: string;
                    provider_auth_generation?: number;
                    provider_account_id?: string;
                    machine_id?: string;
                    cwd?: string;
                    execution_binding?: unknown;
                };
                if (!data.session_id) throw new Error("Session creation returned no id");

                // Known-empty: no history is coming (the agent only starts on the
                // first prompt, and OpenSession doesn't snapshot), so mark it
                // hydrated now — otherwise the transcript skeleton spins forever.
                markSessionHydrated(data.session_id);
                // Apply the title set in the modal. renameSession trims +
                // no-ops on empty, so a cleared title falls back to the
                // daemon default + first-prompt auto-title.
                renameSession(data.session_id, title);
                const sourcePath = selectedWorkspace?.help ?? cwd;
                const trimmedTitle = title.trim();
                onCreated({
                    id: data.session_id,
                    provider,
                    ...(selectedProviderEntry
                        ? {
                            provider_version:
                                data.provider_version || selectedProviderEntry.provider_version,
                            provider_generation_digest:
                                data.provider_generation_digest ||
                                (selectedInstalledProvider?.generation_digest ??
                                    selectedProviderEntry.artifact_digest ?? ""),
                            ...(data.provider_auth_generation !== undefined
                                ? { provider_auth_generation: data.provider_auth_generation }
                                : selectedInstalledProvider?.auth_generation !== undefined
                                ? { provider_auth_generation: selectedInstalledProvider.auth_generation }
                                : {}),
                        }
                        : {}),
                    machine_id: data.machine_id ?? placement.runtimeMachineId,
                    ...(data.provider_account_id ? { provider_account_id: data.provider_account_id } : {}),
                    cwd: data.cwd ?? sourcePath,
                    ...(Object.hasOwn(data, "execution_binding") ? { execution_binding: data.execution_binding } : {}),
                    title: trimmedTitle || `${provider} · ${sourcePath}`,
                    status: "starting",
                    origin: "web",
                    ...(selectedWorkspace
                        ? {
                            workspace_id: selectedWorkspace.value,
                            workspace_name: selectedWorkspace.label,
                            workspace_source_path: selectedWorkspace.help,
                        }
                        : {}),
                }, directory || null);
                created = true;
                onClose();
            } catch (error) {
                // Creating a session is live-only (docs/offline-first-sync.md,
                // class C): a network failure says so in place instead of
                // surfacing the browser's "Failed to fetch".
                const unreachable = error instanceof TypeError ||
                    globalThis.navigator?.onLine === false;
                setCreateError(
                    unreachable
                        ? "Needs a connection. A new session prepares its workspace on the Machine, so try again when Cowboy is reachable."
                        : error instanceof Error
                        ? error.message
                        : "Session creation failed",
                );
            } finally {
                // A created session keeps the sheet submitted while it closes;
                // the next open resets it. Releasing here would let a second
                // activation during the close animation create a duplicate.
                if (!created) {
                    creatingRef.current = false;
                    setCreating(false);
                }
            }
        })();
    };
    // Desktop confirmations consistently require Mod+Enter. Keep bare Enter
    // available to the Select controls in this form; the title field and
    // focused Create button suppress their own native bare-Enter activation.
    // Mobile retains its established single-Enter form behaviour.
    useConfirmEnter(open && desktop && !confirmDiscard, create, { suppressBareEnter: false });
    useConfirmEnter(open && confirmDiscard, discardAndClose);
    // The shared modal grammar (modalNavigation) owns Insert/Normal and hjkl.
    // A composing IME keeps every key: its Esc cancels the marked text, never
    // the modal.
    const onDesktopFormKeyDown = (e: React.KeyboardEvent<HTMLDivElement>): void => {
        if (!desktop || e.defaultPrevented) return;
        if (desktopKeyIntent(e.nativeEvent).owner === "ime" && e.key === "Escape") {
            e.stopPropagation();
        }
    };
    const directoryName = sessionFolderById(sessionDirectories, directory)?.name ?? "directory";
    const projectFromDirectory = placement.hinted.project ||
        Boolean(directoryDefaults.sources.project || directoryDefaults.sources.machine);
    const defaultsSummary = createDefaultsSummary({
        directoryName,
        explicitSources: Object.values(directoryDefaults.sources).map((id) => sessionFolderById(sessionDirectories, id)?.name ?? "directory"),
        hintSession: placement.hinted.project || placement.hinted.installation || subscription.hinted ? sessionHints?.session.title : undefined,
    });
    const saveableDefaults = variant === "session" && directory && placement.ready && placement.project && placement.installation
        ? createDefaultsChanges(directoryDefaults.values, {
            project: placement.project.projectId,
            projectName: placement.project.name,
            machine: placement.project.machineId,
            runtime_machine: placement.installation.runtime_machine_id,
            provider: placement.installation.provider,
            // Only a picked preset is pinned; the agent default stays live.
            preset: presetOverride || undefined,
        })
        : undefined;
    const form = (
            <Stack
                spacing={2}
                sx={{ mt: 1 }}
                onKeyDown={onDesktopFormKeyDown}
                onFocus={(e): void => {
                    setFocusZone(
                        e.target instanceof Element && e.target.closest("[role='tablist']")
                            ? "tabs"
                            : isTextEditingTarget(e.target) ? "text" : "other",
                    );
                }}
                onBlur={(e): void => {
                    if (!(e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget))) {
                        setFocusZone("other");
                    }
                }}
            >
                {createError ? <Alert severity="error">{createError}</Alert> : null}
                {variant === "session" && !presetAvailable && <Alert severity="warning">The inherited preset is unavailable for this agent. Choose a compatible preset or Agent default.</Alert>}
                {!editingFolderDefault && <>
                {!folderOnly && <CreateVariantPicker
                    value={variant}
                    disabled={creating}
                    keyboard={desktop}
                    digitsAvailable={focusZone !== "text"}
                    onChange={(next, source): void => {
                        if (source === "roving") {
                            // A keyboard pick (digit, H/L) keeps the Normal
                            // cursor where it is; I edits the title.
                            setVariant(next);
                            setCreateError("");
                            return;
                        }
                        // Flush the new title while still inside the tap gesture:
                        // WebKit can raise the keyboard only for this synchronous focus.
                        globalThis.clearTimeout(titleFocusTimer.current);
                        const select = next === "session" || (next === "draft" && selectDraftTitleOnFocus.current);
                        flushSync(() => {
                            setVariant(next);
                            setCreateError("");
                        });
                        const input = titleRef.current;
                        input?.focus({ preventScroll: true });
                        if (input && select) {
                            if (next === "draft") selectDraftTitleOnFocus.current = false;
                            input.select();
                            requestAnimationFrame(() => {
                                if (document.activeElement === input) input.select();
                            });
                        }
                    }}
                />}
                {desktop ? (
                    // The tablist's status line: it names only the keys of the
                    // current layer and keeps its height so focus never shifts
                    // the form.
                    <Stack
                        data-create-key-hint={focusZone}
                        direction="row"
                        alignItems="center"
                        flexWrap="wrap"
                        sx={{ mt: "4px !important", minHeight: 20, px: 1, color: "text.secondary", typography: "caption" }}
                    >
                        <DesktopModalKeyHint mode={focusZone === "text" ? "insert" : "normal"} tabs={folderOnly ? 0 : 3} />
                    </Stack>
                ) : null}
                </>}
                <Stack spacing={1.5} id="create-variant-panel" role="tabpanel" aria-labelledby={folderOnly || editingFolderDefault ? undefined : `create-${variant}-tab`} aria-label={folderOnly || editingFolderDefault ? "Folder settings" : undefined}>
                    <Stack spacing={1.5} sx={{ display: editingFolderDefault ? "none" : "flex" }}>
                    {variant === "session" && placement.error ? <Alert severity="error">{placement.error}</Alert> : null}
                    <TextField
                        size={desktop ? "small" : "medium"}
                        label={variant === "folder" ? "Folder name" : "Title"}
                        value={variant === "draft" ? draftTitle : variant === "folder" ? folderName : title}
                        onChange={(e): void => {
                            if (variant === "draft") {
                                selectDraftTitleOnFocus.current = false;
                                setDraftTitle(e.target.value);
                            } else if (variant === "folder") setFolderName(e.target.value);
                            else setTitle(e.target.value);
                        }}
                        // A folder is not a contact: without this iOS reads
                        // "name" and raises the AutoFill Contact bar
                        // (FolderNameShell, physical iPad 2026-09-17).
                        autoComplete={variant === "folder" ? "off" : undefined}
                        name={variant === "folder" ? "cowboy-session-folder" : undefined}
                        disabled={creating}
                        inputRef={titleRef}
                        autoFocus
                        onFocus={(e): void => {
                            // Select the whole default ("New session N") on EVERY focus, so
                            // tapping the field replaces it in one go. Deferred a frame — iOS
                            // collapses a synchronous select() back to a caret. Same logic as
                            // the session-rename field (Composer.tsx).
                            const input = e.target as HTMLInputElement;
                            if (variant === "session" || (variant === "draft" && selectDraftTitleOnFocus.current)) {
                                if (variant === "draft") selectDraftTitleOnFocus.current = false;
                                requestAnimationFrame(() => {
                                    if (document.activeElement === input) input.select();
                                });
                            }
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
                        placeholder={variant === "draft" ? "Untitled" : variant === "folder" ? "Name this folder" : "Name this session"}
                        helperText={variant === "draft"
                            ? "Device local time · replace this name or rename it while editing"
                            : variant === "folder"
                            ? "A shared home for sessions and drafts."
                            : "Clear to auto-name from the first message"}
                    />
                    {variant === "draft" ? (
                        <DraftCreationDirectory value={directory} onChange={setDirectory} />
                    ) : variant === "folder" ? <>
                        <WorkspacePicker
                            label="Inside folder (optional)"
                            clearable
                            hierarchyPreferenceKey="cowboy.sessionDirectoryHierarchy"
                            entries={directoryChoices}
                            value={directory}
                            onChange={setDirectory}
                        />
                    </> : <>
                        {/* The directory decides every default below, so it is chosen first. */}
                        <WorkspacePicker
                            label="Directory (optional)"
                            clearable
                            hierarchyPreferenceKey="cowboy.sessionDirectoryHierarchy"
                            entries={directoryChoices}
                            value={directory}
                            onChange={setDirectory}
                        />
                        {directory && (
                            <Stack direction="row" alignItems="center" flexWrap="wrap" data-create-defaults-summary sx={{ gap: 1, mt: "4px !important" }}>
                                <Typography variant="caption" color="text.secondary" sx={{ mr: "auto" }}>
                                    {defaultsSummary}
                                </Typography>
                                {saveableDefaults && (
                                    <Button size="small" disabled={creating} onClick={(): void => setSessionFolderDefaults(directory, saveableDefaults)}>
                                        Save as {directoryName} defaults
                                    </Button>
                                )}
                            </Stack>
                        )}
                        <WorkspacePicker
                            label="Project"
                            entries={placement.projects}
                            value={placement.project?.value ?? ""}
                            defaultValue={placement.defaultProjectValue}
                            configuredDefault={placement.configuredDefaultProject}
                            // A directory-supplied project is not this device's default.
                            onDefaultChange={projectFromDirectory ? undefined : placement.setDefaultProject}
                            onChange={(value): void => {
                                placement.selectProject(value);
                                setWorkItemId("");
                            }}
                        />
                        <AiInstallationPicker
                            installations={placement.installations}
                            value={placement.installation?.value ?? ""}
                            onChange={placement.selectInstallation}
                            helperText={placement.loading ? "Checking available AI installations…" : placement.installation
                                ? `${placement.separate ? "Remote" : "Local"} · AI on ${placement.installation.machine.display_name} · Files and commands on ${machines.find((m) => m.id === machineId)?.display_name ?? machineId}`
                                : fullInstallation
                                ? `${fullInstallation.machine.display_name} is full (${installationCapacity(fullInstallation)}). Choose another AI installation.`
                                : placement.installations.length ? "Choose an available AI installation; the preferred Machine is unavailable."
                                : "No ready AI installation can use this project. Check Machines in Settings."}
                        />
                        <SessionSubscription state={subscription} disabled={creating} desktop={desktop} helperText={subscription.hinted ? `Same as “${sessionHints?.session.title ?? "latest session"}”` : undefined} />
                        <SessionContextPicker value={contextSources} onChange={setContextSources} disabled={creating} />
                        <Typography variant="caption" color="text.secondary">
                            Git projects open in a session worktree. Other directories are shared in place.
                        </Typography>
                        <SessionPresetPicker provider={provider} presets={selectedProviderEntry?.manifest.configuration.presets ?? []} value={selectedPreset} onChange={setPresetOverride} helperText={presetOverride === undefined && directoryDefaults.sources.preset ? `Inherited from ${sessionFolderById(sessionDirectories, directoryDefaults.sources.preset)?.name ?? "directory"}` : "Applied before the first prompt"} />
                        {selectedWorkspace && selectedWorkspace.active_work_items.length > 0 ? (
                            <TextField
                                select
                                label="Durable work item"
                                slotProps={{ select: { displayEmpty: true }, inputLabel: { shrink: true } }}
                                value={workItemId}
                                onChange={(e): void => setWorkItemId(e.target.value)}
                                helperText="Optional · resumes durable context in the selected Provider runtime"
                            >
                                <MenuItem value="">New task</MenuItem>
                                {selectedWorkspace.active_work_items.map((item) => (
                                    <MenuItem key={item.id} value={item.id}>
                                        {item.title}{item.blocked ? " · blocked" : ""}
                                    </MenuItem>
                                ))}
                            </TextField>
                        ) : null}
                    </>}
                    </Stack>
                    {variant === "folder" && <>
                        {!editingFolderDefault && folderPresetMismatch && <Alert severity="warning" action={<Button onClick={(): void => { setFolderDefaultsExpanded(true); setFolderDefaultField("preset"); }}>Choose preset</Button>}>The preset belongs to another agent. Choose a compatible preset or use the agent default.</Alert>}
                        <FolderCreationDefaults
                            name={folderName}
                            parent={directory || null}
                            defaults={folderDefaults}
                            onChange={setFolderDefaults}
                            field={folderDefaultField}
                            onFieldChange={setFolderDefaultField}
                            expanded={folderDefaultsExpanded}
                            onExpandedChange={setFolderDefaultsExpanded}
                            tree={sessionDirectories}
                            machines={machines}
                            catalog={folderCatalog}
                            error={folderCatalogError}
                        />
                    </>}
                </Stack>
            </Stack>
    );
    const canCreate = !creating && (editingFolderDefault || variant === "draft" ||
        (variant === "folder" ? normalizeSessionFolderName(folderName) !== null && !folderPresetMismatch
            : subscription.allowed && placement.ready && providerAvailable(provider) && presetAvailable && Boolean(provider && machineId && cwd)));
    const createLabel = editingFolderDefault ? "Back to folder" : variant === "draft" ? "Create draft" : variant === "folder" ? "Create folder" : "Create session";
    const dialogTitle = editingFolderDefault ? FOLDER_DEFAULT_FIELDS[folderDefaultField!] : folderOnly ? "New folder" : "Create";
    // The cover sheet is the touch presentation. A Desktop window below the
    // lg breakpoint is still Desktop: it keeps the dialog whose Esc, Mod+Enter
    // and leader layers the keyboard contract relies on.
    const discardSheet = (
        <ConfirmSheet
            open={confirmDiscard}
            onClose={(): void => setConfirmDiscard(false)}
            title={variant === "folder" ? "Discard this folder?" : variant === "draft" ? "Discard this draft?" : "Discard this session?"}
            actions={
                <>
                    <Button color="inherit" onClick={(): void => setConfirmDiscard(false)}>
                        Keep editing
                        {desktop && <Kbd keys="Esc" />}
                    </Button>
                    <Button color="error" variant="contained" onClick={discardAndClose}>
                        Discard
                        {desktop && <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />}
                    </Button>
                </>
            }
        >
            <Typography variant="body2" color="text.secondary">Your changes will be lost.</Typography>
        </ConfirmSheet>
    );
    if (navbarAtBottom && !desktop) {
        return (
            <>
            <DetentSheet
                open={open}
                onClose={creating ? (): void => {} : closeLevel}
                cover
                frosted
                ariaLabel={dialogTitle}
                keyboardOpen={keyboardOpen}
                surfaceColor={theme.palette.background.default}
                footer={
                    <Box data-new-session-footer-actions sx={{ width: "100%" }}>
                        <MobileDecisionActions
                            shelf
                            flat
                            onCancel={creating ? (): void => {} : requestClose}
                            cancelDisabled={creating}
                            confirmLabel={creating ? "Creating…" : createLabel}
                            onConfirm={create}
                            confirmDisabled={!canCreate}
                            confirmBusy={creating}
                            keyboardOpen={keyboardOpen}
                            preserveFocus
                        />
                    </Box>
                }
                header={
                    <Box sx={{ pl: 2, pr: 2, pb: 1 }}>
                        <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>
                            {dialogTitle}
                        </Typography>
                    </Box>
                }
            >
                <Box sx={{ px: 2 }}>
                    {form}
                </Box>
            </DetentSheet>
            {discardSheet}
            </>
        );
    }
    return (
        <>
        <Sheet
            open={open}
            onClose={creating ? (): void => {} : closeLevel}
            title={dialogTitle}
            wide={desktop}
            actions={
                <>
                    <Button onClick={requestClose} color="inherit" disabled={creating}>
                        Cancel
                        {!editingFolderDefault && <Kbd keys="Esc" availability={focusZone === "text" ? "inactive" : "available"} />}
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
                        disabled={!canCreate}
                    >
                        {creating ? "Creating…" : createLabel}
                        <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
                    </Button>
                </>
            }
        >
            {form}
        </Sheet>
        {discardSheet}
        </>
    );
}

// Prefills the textfield with the current title; Save is disabled while empty
// or unchanged (server-side also rejects empty). The real Title input is mounted
// and focused synchronously inside the rename tap below.

// The connecting/loading placeholder — SKELETONS that mirror the real layout
// (a settling list of items in the sidebar, a settling transcript in the main
// pane) instead of a bare spinner: it reads as "content arriving", not "stuck".
// If the WS snapshot still hasn't landed after a stall (a wedged socket / SW), it
// reveals a manual reload so a session never sits dead forever. NOT auto-reload:
// an auto-loop could spin if the reload also stalls.
export function LoadingState({ compact = false }: { compact?: boolean }): React.JSX.Element {
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
            <Stack data-boot-loading spacing={0.75} sx={{ px: 1, py: 1 }} aria-label="Loading sessions">
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
            // Never save a screen that is itself a placeholder (bootSnapshot.ts).
            data-boot-loading
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

// --- Info: per-session dialog + the Settings storage panel ------------------
// Both read the daemon's HTTP info endpoints (GET, no WS) and surface the
// capacity numbers the audit flagged — events are the unbounded grower.

// A label/value row used by both info surfaces.
export function InfoRow({ k, v }: { k: string; v: string }): React.JSX.Element {
    return (
        <Stack direction="row" spacing={2} sx={{ justifyContent: "space-between", alignItems: "baseline" }}>
            <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0 }}>{k}</Typography>
            <Typography variant="body2" sx={{ wordBreak: "break-all", textAlign: "right" }}>{v}</Typography>
        </Stack>
    );
}

export function SessionOverviewRows(
    { info, live }: { info: SessionInfoPayload; live?: SessionMeta | null },
): React.JSX.Element {
    const { data: accounts } = useSubscriptionAccounts();
    const merged = mergeSessionOverview(info, live);
    const sections = sessionOverviewSections(merged);
    const account = sessionAccountIdentity(accounts, merged);
    sections[0]?.rows.splice(3, 0, { label: "Account", value: account.label });
    return (
        <Stack spacing={1.15} sx={{ mt: 1 }}>
            {sections.map((section, index) => (
                <Fragment key={section.key}>
                    {index > 0 && <Divider />}
                    {section.rows.map((row) => (
                        <InfoRow key={row.label} k={row.label} v={row.value} />
                    ))}
                </Fragment>
            ))}
        </Stack>
    );
}

export function DesktopSessionInfoPanel(
    { session }: { session: SessionMeta | null },
): React.JSX.Element {
    const [info, setInfo] = useState<SessionInfoPayload | null>(null);
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
                    ? response.json() as Promise<SessionInfoPayload>
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
                : <SessionOverviewRows info={info} live={session} />}
        </Box>
    );
}
