import { desktopSize } from "./surface/desktopSize";
import { DeleteSessionShell } from "./DeleteSessionShell";

import { draftRepository } from "./documents/store";
import { openDrafts } from "./documents/navigation";

import { useDraftRoute, leaveDrafts } from "./documents/navigation";
import { useDraftLibrary } from "./documents/store";
import { draftMissingDirectory } from "./documents/model";
import type { DraftFlush } from "./documents/DraftEditor";
import { documentNotice } from "./documents/DocumentNotifications";
import { movedToTrash } from "./trashFeedback";
import { UNDO_NOTICE_MS } from "./undoHistory";

import { WorkspaceDraftPane, WorkspaceDraftActions, type WorkspaceDraftAction } from "./documents/WorkspaceDraft";

import { liveDrafts, sessionIsWorking } from "./trashModel";

import { copyDraftToSession } from "./documents/transfer";

import { PriorSendDecisionSheet } from "./PriorSendDecisionSheet";

import { forwardRef, lazy, memo, Suspense, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import type { ComponentProps } from "react";
import { flushSync } from "react-dom";
import { Alert, alpha, AppBar, Box, Button, IconButton, Snackbar, Stack, TextField, Toolbar, Tooltip, Typography, useMediaQuery, useTheme } from "@mui/material";

import { Add, KeyboardDoubleArrowLeft, ViewListOutlined, Close as CloseIcon, Code as CodeIcon, Menu as MenuIcon, MoreVert, DescriptionOutlined } from "@mui/icons-material";
import { SessionControls } from "./Composer";
import { SessionReloadDialog } from "./SessionReloadDialog";
import { MobileComposer } from "./mobile/MobileComposer";
import { openMobileProduct } from "./mobile/appPagerMotion";
import {
    OPEN_APP_SETTINGS_EVENT,
    appSettingsFromEvent,
    openAppSettings,
    type SettingsProductFocus,
} from "./appSettings";

import { useClientUpdateSettings } from "./clientUpdateSettings";

import { claimKeyboard } from "./keyboardClaim";

import { setObservabilityContext } from "./observability";
import { Transcript } from "./Transcript";
import { sessionProjectLabel } from "./sessionProject";

import { PICK_SESSION_EVENT, pickSessionDetail } from "./sessionPickRequest";

import { type SessionInfoPayload } from "./sessionOverview";
import {
    useProviderCatalog,
} from "./providerCatalog";
import { sessionProviderAuthShortcut } from "./providerAuthShortcut";
import { shouldShowSessionErrorSnackbar } from "./sessionErrorNotice";

import { type ConfigOption, type Envelope, type SessionMeta, type Status } from "./protocol";
import { StatusDot } from "./SessionStatusDot";
import { currentConfigOptionName, providerConfigOptions } from "./providerConfigOptions";
import { canApplyUpdateNow, deleteSession, stopAndTrashSession, trashItems, openSession, releaseInactiveHistory, renameSession, useStoreSelector } from "./store";

import { buildSessionTree, displayedSessionOrder } from "./sessionTree";
import { SESSION_FOLDER_SHEET_HOST } from "./SessionFolderUi";

import { useDialogInputFocus } from "./useDialogInputFocus";
import { useSheetKeyboardDiagnostics } from "./sheetKeyboardDiagnostics";

import { sessionsRailGroups } from "./desktop/sessionsRailGroups";

import { useReliableTouchTap } from "./useReliableTouchTap";
import { useBackdropDismiss } from "./useBackdropDismiss";
import { useMobileDrawerSurface } from "./mobileDrawerSurface";
import { mobileSpatialDrawerShadow } from "./mobileDrawerDepth";
import { MOBILE_SESSION_DRAWER_WIDTH } from "./mobileDrawerMotion";
import {
    mobileDrawerRailHitSx,
    mobilePeekRestLayerSx,
    mobilePresentationMovingRootSx,
} from "./mobilePresentationMotion";
import { frostedChrome, frostedStatusChrome } from "./frostedGlass";
import { readerTopEdge } from "./readerTopEdge";
import { clampComposerColWidth, composerColWidthStore, DESKTOP_COMPACT_WIDTH_QUERY, DESKTOP_CONVERSATION_MIN, DESKTOP_PROMPT_MIN, clampPromptRatio, collapsedSessionsPromptRatioStore, dragCollapses } from "./desktopLayout";

import { useNavbarAtBottom } from "./navbarSettings";
import { mobileComposerFocusMotion } from "./mobileComposerPrimitives";
import { MOBILE_NOTICE_BOTTOM, useFloatingComposerGeometry } from "./floatingComposerGeometry";
import { releaseMobileComposerFocus } from "./composer/mobileComposerFocus";

import { ProviderIcon } from "./ProviderIcon";

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
import { ConnectionBanner, DetentSheet, MobileSheetDismiss, NativeReleaseUpdatePrompt } from "@cowboy/app-shell";
import { markUpdateSwapping } from "./updateAttempt";
import { Sheet } from "./Sheet";

// Desktop has no deployed-version string to name, only the fact of the swap.
// Module scope keeps the prop identity stable across renders.
function markDesktopUpdateSwapping(): void {
    markUpdateSwapping(globalThis.localStorage, "unknown", Date.now());
}

import { Kbd, useConfirmEnter } from "./Kbd";

import { HintTooltip } from "./HintTooltip";

import { isImeKeyEvent } from "./imeKey";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { DESKTOP_SHORTCUTS } from "./desktop/commands/workspaceShortcuts";

import { desktopRecentItems, recordDesktopVisit, useDesktopVisits } from "./desktop/sessionVisits";
import { pruneDesktopTabs, updateDesktopTabs, useDesktopTabs, withDesktopTab } from "./desktop/workspaceTabs";
import type { DesktopTabItem } from "./desktop/DesktopTabStrip";

import { DesktopShortcut, desktopShortcutCaps } from "./desktop/commands/DesktopKeycap";

import { ResourceLightbox } from "./ResourceLightbox";
import { desktopFocusBoundary, desktopFocusFill, type Mode as ThemeMode } from "./theme";

import { useSurfaceProfile } from "./surface/SurfaceProfile";

import {
    controlPlaneConnection,
    getActiveSessionId,
    setActiveSessionId,
    useActiveSessionId,
} from "./controlPlane";
import { resolveActiveSession } from "./sessionSelection";
import { useSessionFocusLifecycle } from "./sessionFocusLifecycle";
import { useBootPresentation } from "./useBootPresentation";

import { useOptionalDesktopWorkspace } from "./desktop/DesktopWorkspaceController";

import { SettingsShell, type SettingsTab, type ProviderSettingsIntent, type ProviderSettingsTarget } from "./SettingsShell";
import { SessionList, NO_COLLAPSED_FOLDERS, DesktopSidebarAction, CURRENT_ROW_FILL, CreateDialog, LoadingState, loadDesktopPaneCollapse, SessionOverviewRows, desktopPaneCollapseModule } from "./SessionList";
import { useDesktopSplitters, SIDEBAR_MIN, SIDEBAR_MAX, clampSidebarWidth, sidebarWidthStore } from "./desktopSplitters";
export { CreateDialog } from "./SessionList";

const DesktopCommandHost = lazy(async () => {
    const module = await import("./desktop/commands/DesktopCommandHost");
    return { default: module.DesktopCommandHost };
});
const DesktopStatusLine = lazy(async () => {
    const module = await import("./desktop/DesktopStatusLine");
    return { default: module.DesktopStatusLine };
});
const DesktopTabStrip = lazy(async () => {
    const module = await import("./desktop/DesktopTabStrip");
    return { default: module.DesktopTabStrip };
});
const DesktopTopBarControls = lazy(async () => {
    const module = await import("./desktop/DesktopTopBarControls");
    return { default: module.DesktopTopBarControls };
});
const DesktopComposer = lazy(async () => {
    const module = await import("./desktop/DesktopComposer");
    return { default: module.DesktopComposer };
});

const DesktopWorkspace = lazy(async () => {
    const module = await import("./desktop/DesktopWorkspace");
    return { default: module.DesktopWorkspace };
});
const DesktopRegionShortcut = lazy(async () => {
    const module = await import("./desktop/DesktopRegionShortcut");
    return { default: module.DesktopRegionShortcut };
});

const LazyDesktopSessionsRail = lazy(async () => {
    const module = await loadDesktopPaneCollapse();
    return { default: module.DesktopSessionsRail };
});
function DesktopSessionsRail(
    props: ComponentProps<typeof LazyDesktopSessionsRail>,
): React.JSX.Element {
    // Fixed at mount: swapping component types later would remount the rail.
    const [Rail] = useState(() =>
        desktopPaneCollapseModule?.DesktopSessionsRail ?? LazyDesktopSessionsRail
    );
    return <Rail {...props} />;
}
const DesktopSplitterHint = lazy(async () => {
    const module = await import("./desktop/DesktopSplitterHint");
    return { default: module.DesktopSplitterHint };
});

// The activity bar's entries need about 12.5rem with their slots; below 13rem
// (a narrow sidebar at a large font size) they keep only their glyphs.
const SESSIONS_ACTIVITY_SLOT_FIT_SX = {
    containerType: "inline-size",
    "@container (max-width: 13rem)": {
        "& .cowboy-sidebar-slot": { display: "none !important" },
    },
} as const;
const DesktopRailGlobalActions = lazy(async () => {
    const module = await import("./desktop/DesktopGlobalActions");
    return { default: module.DesktopRailGlobalActions };
});
const DesktopExtensionsActivity = lazy(async () => {
    const module = await import("./desktop/DesktopGlobalActions");
    return { default: module.DesktopExtensionsActivity };
});
const DesktopSettingsAction = lazy(async () => {
    const module = await import("./desktop/DesktopGlobalActions");
    return { default: module.DesktopSettingsAction };
});

const EMPTY_CONFIG_OPTIONS: ConfigOption[] = [];

function EmptyWorkspaceState({
    onNewSession,
}: {
    onNewSession: () => void;
}): React.JSX.Element {
    const desktop = useSurfaceProfile().kind === "desktop";
    return (
        <>
            <Typography color="text.secondary">No session selected.</Typography>
            <Button
                variant="contained"
                startIcon={<Add />}
                onClick={onNewSession}
            >
                Create
                {desktop && <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.newSession} quiet />}
            </Button>
        </>
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
    const surface = useSurfaceProfile();
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
    const session = useStoreSelector((snapshot) =>
        snapshot.sessions.find((candidate) => candidate.id === sessionId)
    );
    const rawConfigOptions = useStoreSelector((snapshot) =>
        snapshot.configOptions.get(sessionId) ?? EMPTY_CONFIG_OPTIONS
    );
    const configOptions = providerConfigOptions(provider, rawConfigOptions);
    const model = configOptions.find((option) => option.id === "model");
    const reasoning = configOptions.find((option) =>
        option.id === "effort" || option.id === "reasoning_effort"
    );
    const emptyContext = {
        provider,
        providerVersion: session?.provider_version,
        providerDigest: session?.provider_generation_digest,
        project: session ? sessionProjectLabel(session) : null,
        machine: session?.machine_id ?? null,
        model: currentConfigOptionName(model),
        reasoning: currentConfigOptionName(reasoning),
    };
    const { projection, transitionAnchorKey } = useExploreSessionState(sessionId);

    return projection === "explore" ? (
        <ExploreTranscript
            desktop={desktopNavigation}
            sessionId={sessionId}
            timeline={timeline ?? EMPTY_TRANSCRIPT_TIMELINE}
            status={status}
            provider={provider}
            providerVersion={session?.provider_version}
            providerDigest={session?.provider_generation_digest}
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
            providerVersion={session?.provider_version}
            providerDigest={session?.provider_generation_digest}
            cwd={cwd}
            loading={!hydrated}
            connected={connected}
            emptyContext={emptyContext}
            topInset={topInset}
            bottomInset={bottomInset}
            onScrollableChange={onScrollableChange}
            // A short phone transcript belongs beside the composer: the next
            // action and its context stay in one thumb-sized region. On a tall
            // tablet that same bottom anchor creates a conspicuous empty upper
            // half, so tablet History follows the document-like Desktop/Page
            // treatment and starts below the top chrome instead.
            shortContentAtTop={desktopNavigation || surface.kind === "tablet"}
            restoreAnchorKey={transitionAnchorKey}
            onAnchorRestored={(): void => resolveProjectionAnchor(sessionId)}
        />
    );
});

type SettingsControllerHandle = {
    open: (
        tab: SettingsTab,
        section?: SettingsProductFocus,
        providerTarget?: ProviderSettingsTarget,
    ) => void;
};

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
    const draftRoute = useDraftRoute();
    const draftLibrary = useDraftLibrary();
    useEffect(() => {
        if (draftRoute.active && !draftRoute.id && draftLibrary.loaded) {
            const first = draftLibrary.entries.find((entry) => entry.kind === "document" && !entry.deleted);
            if (first) openDrafts(first.id);
            else leaveDrafts();
        }
    }, [draftRoute.active, draftRoute.id, draftLibrary.loaded, draftLibrary.entries]);
    // Deleting a directory moves its Drafts on the server without a push:
    // fetch the result once the deletion has had time to persist.
    const workspaceDirectories = useStoreSelector((snapshot) => snapshot.sessionFolders.folders);
    const draftDirectoryGone = useMemo(
        () => draftMissingDirectory(draftLibrary.entries, new Set(workspaceDirectories.map((folder) => folder.id))),
        [draftLibrary.entries, workspaceDirectories],
    );
    useEffect(() => {
        if (!draftDirectoryGone) return;
        const timer = globalThis.setTimeout(() => void draftRepository().refresh(), 1000);
        return () => globalThis.clearTimeout(timer);
    }, [draftDirectoryGone, workspaceDirectories]);
    const draftBeforeLeave = useRef<DraftFlush>(() => Promise.resolve());
    const [draftAction, setDraftAction] = useState<WorkspaceDraftAction | null>(null);
    // Load the signed Provider catalog once at the app boundary so every
    // presentation helper reads the same dynamic identity registry.
    const { catalog: providerCatalog } = useProviderCatalog();
    const clientUpdateSettings = useClientUpdateSettings();
    const desktopWorkspace = useOptionalDesktopWorkspace();
    const sessions = useStoreSelector((snapshot) => snapshot.sessions);
    const trashedSessions = useStoreSelector((snapshot) => snapshot.trashedSessions);
    const lastError = useStoreSelector((snapshot) => snapshot.lastError);
    const sessionsLoaded = useStoreSelector((snapshot) => snapshot.sessionsLoaded);
    // `replica` paints the last known list before the socket answers; only a
    // `live` list is authoritative for pruning and "session gone" notices.
    const sessionsSource = useStoreSelector((snapshot) => snapshot.sessionsSource);
    // The error notice is monotonically `seq`-stamped so the same message
    // text triggers the snackbar twice if it happens again. Tracking the
    // `seq` we've shown means we don't re-open after the user dismisses.
    const [shownErrorSeq, setShownErrorSeq] = useState(0);
    const providerAuthShortcut = useMemo(
        () => sessionProviderAuthShortcut(lastError, sessions, providerCatalog),
        [lastError, providerCatalog, sessions],
    );
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
    const compactDesktopWidth = useMediaQuery(DESKTOP_COMPACT_WIDTH_QUERY);
    const desktopNavCollapsed = surface === "desktop" && compactDesktopWidth;
    const sessionsInDrawer = mobile || desktopNavCollapsed;
    // A user-collapsed Sessions pane (wide Desktop only) keeps its list mounted
    // but hidden and shows the narrow session switcher rail instead. Compact
    // Desktop already owns Sessions through the drawer above.
    const sessionsCollapsed = surface === "desktop" && !compactDesktopWidth &&
        desktopWorkspace?.collapsedPanes.sessions === true;
    // Navbar placement belongs exclusively to the Touch product. When its user
    // picks "bottom", the AppBar moves below the transcript, just
    // above the composer (mobile-browser bottom-bar feel). The modals read the
    // same flag and force their bottom-sheet surface (see BottomSheet
    // `forceSheet`), so a tablet's bottom navbar gets bottom-up modals too
    // rather than centered dialogs.
    const prefersNavbarAtBottom = useNavbarAtBottom();
    const navbarAtBottom = mobile && prefersNavbarAtBottom;
    const floatingPanelHeight = "var(--floating-stack-h, 0px)";
    // One geometry owner measures the complete floating bottom stack. The live
    // frosted material and the settled Transcript inset consume its two outputs;
    // no child panel publishes an independent reservation.
    const columnRef = useRef<HTMLDivElement>(null);
    const mobileLayerRef = useRef<HTMLDivElement>(null);
    const mobilePageRef = useRef<HTMLDivElement>(null);
    const mobileNavFollowRef = useRef<HTMLElement | null>(null);
    const mobileComposerFollowRef = useRef<HTMLDivElement | null>(null);
    const mobileFrostFollowRef = useRef<HTMLDivElement | null>(null);
    const mobileStatusChromeFollowRef = useRef<HTMLDivElement | null>(null);
    const { appBarRef, composerRef } = useFloatingComposerGeometry({
        surfaceRef: columnRef,
        navbarAtBottom,
    });
    // React detaches and reattaches a changed callback ref even when the DOM
    // node survives. Streamed session updates must not reset the geometry
    // observer, clear its disclosure hold, or publish a temporary zero height.
    const attachAppBar = useCallback((element: HTMLElement | null): void => {
        appBarRef(element);
        mobileNavFollowRef.current = mobile && navbarAtBottom ? element : null;
    }, [appBarRef, mobile, navbarAtBottom]);
    const attachComposer = useCallback((element: HTMLDivElement | null): void => {
        composerRef(element);
        mobileComposerFollowRef.current = mobile ? element : null;
    }, [composerRef, mobile]);
    const activeId = useActiveSessionId();
    const errorOpen = shouldShowSessionErrorSnackbar(
        lastError,
        activeId,
        shownErrorSeq,
    );
    const setActiveId = setActiveSessionId;
    const observabilityMachineId = sessions.find((session) => session.id === activeId)?.machine_id;
    useEffect(() => {
        setObservabilityContext({
            ...(activeId ? { session_id: activeId } : {}),
            ...(observabilityMachineId ? { machine_id: observabilityMachineId } : {}),
        });
    }, [activeId, observabilityMachineId]);
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
        return () => onMobileDrawerOpenChange?.(false);
    }, [mobile, onMobileDrawerOpenChange]);
    const mobileShellRef = useRef<HTMLDivElement>(null);
    const mobileDrawerRef = useRef<HTMLDivElement>(null);
    const mobileDrawerMaskRef = useRef<HTMLDivElement>(null);
    const mobileDrawerDimRef = useRef<HTMLDivElement>(null);
    const settleMobileDrawerRef = useRef<(
        (
            open: boolean,
            releaseVelocity?: number,
            onSettled?: () => void,
            cachedWidth?: number,
        ) => void
    ) | null>(null);
    const closeSessionsPeek = useBackdropDismiss<HTMLDivElement>((): void => {
        if (drawerOpen) settleMobileDrawerRef.current?.(false);
    });
    const [dialogOpen, setDialogOpen] = useState(false);
    const openNewSession = (): void => {
        // Preserve the in-gesture keyboard claim until Create's Title mounts.
        if (mobile) claimKeyboard();
        setDialogOpen(true);
    };
    const [pendingCreatedSession, setPendingCreatedSession] = useState<SessionMeta | null>(null);
    const settingsControllerRef = useRef<SettingsControllerHandle>(null);
    const [exploreComposeIntent, setExploreComposeIntent] = useState<{
        kind: "follow_up" | "new_page";
        targetPageId: string | null;
        knownPageIds: string[];
    } | null>(null);

    useMobileDrawerSurface({
        mobile,
        phone,
        onMobileDrawerOpenChange,
        setDrawerOpen,
        columnRef,
        mobileLayerRef,
        mobilePageRef,
        mobileNavFollowRef,
        mobileComposerFollowRef,
        mobileFrostFollowRef,
        mobileStatusChromeFollowRef,
        mobileShellRef,
        mobileDrawerRef,
        mobileDrawerMaskRef,
        mobileDrawerDimRef,
        drawerOpenRef,
        settleMobileDrawerRef,
    });

    // Settings + Info are one merged sheet. Its open state lives in an isolated
    // controller so mounting a cover sheet does not re-render the transcript,
    // CodeMirror composer, session rail, and all of their layout effects.
    const openSettings = useCallback((
        tab: SettingsTab,
        section?: SettingsProductFocus,
        providerTarget?: ProviderSettingsTarget,
    ): void => {
        settingsControllerRef.current?.open(tab, section, providerTarget);
    }, []);
    const openCodeTap = useReliableTouchTap<HTMLButtonElement>((): void => {
        releaseMobileComposerFocus();
        if (drawerOpenRef.current) {
            drawerOpenRef.current = false;
            onMobileDrawerOpenChange?.(false);
            settleMobileDrawerRef.current?.(false);
        }
        openMobileProduct("review");
    });
    // The docked Sessions title bar hosts the list's Create and fold actions.
    const [sessionsHeaderActions, setSessionsHeaderActions] = useState<HTMLElement | null>(null);
    const {
        sidebarWidth,
        setSidebarWidth,
        resizing,
        setResizing,
        widthRef,
        splitActive,
        colWidth,
        setColWidth,
        colResizing,
        setColResizing,
        colWidthRef,
        sessionsCollapseIntent,
        setSessionsCollapseIntent,
        workCollapseIntent,
        setWorkCollapseIntent,
        promptRatio,
        setPromptRatio,
        promptRatioRef,
    } = useDesktopSplitters({ surface, sessionsCollapsed, desktopWorkspace, setDrawerOpen });
    // Session targeted for deletion; non-null = the dialog is open. Held in
    // App so the dialog is a single instance instead of one per row, and so
    // its mobile vs desktop shell can react to `bottomSheet` from useMediaQuery.
    const [pendingDelete, setPendingDelete] = useState<SessionMeta | null>(
        null,
    );
    // Desktop deletion always asks first; Mobile retains immediate idle Trash.
    const requestDelete = (session: SessionMeta): void => {
        if (surface === "desktop" || sessionIsWorking(session)) {
            setPendingDelete(session);
            return;
        }
        movedToTrash(session, trashItems({ sessions: [session.id] }));
    };
    // Same pattern for rename — single dialog instance prefilled with the
    // session's current title; Mobile/desktop shell split mirrors the rest.
    // Folder a "New session here" request files its created session into.
    const newSessionFolderRef = useRef<string | null>(null);
    const [pendingRename, setPendingRename] = useState<SessionMeta | null>(
        null,
    );
    // Per-session info dialog target (kebab → Info).
    const [pendingInfo, setPendingInfo] = useState<SessionMeta | null>(null);
    // Runtime reload is deliberately separate from Clear: it keeps the same
    // native thread and all Cowboy-owned history/config/pending state.
    const [pendingReload, setPendingReload] = useState<SessionMeta | null>(null);
    // The POST response can beat the independent WS `sessions` broadcast. Keep
    // the created session visible and selected during that gap; once the
    // authoritative row arrives, the projection naturally stops being used.
    const sessionsForView = pendingCreatedSession &&
            !sessions.some((session) => session.id === pendingCreatedSession.id)
        ? [pendingCreatedSession, ...sessions]
        : sessions;
    // The collapsed Sessions rail shows the folder structure (fully expanded,
    // independent of this device's folds).
    const railFolders = useStoreSelector((snapshot) => snapshot.sessionFolders);
    const railTrashFolders = useStoreSelector((snapshot) => snapshot.trashFolders);
    const workspaceOrder = useStoreSelector((snapshot) => snapshot.workspaceOrder);
    const collapsedRail = useMemo(() => {
        if (!sessionsCollapsed) return { groups: [] };
        const ordered = displayedSessionOrder(sessionsForView);
        const rows = buildSessionTree(ordered, railFolders, NO_COLLAPSED_FOLDERS, liveDrafts(draftLibrary.entries, railTrashFolders), workspaceOrder).rows;
        return {
            groups: sessionsRailGroups(rows, draftRoute.id ? `draft:${draftRoute.id}` : activeId),
        };
    }, [activeId, railFolders, railTrashFolders, sessionsCollapsed, sessionsForView, draftLibrary.entries, draftRoute.id, workspaceOrder]);
    const active = draftRoute.active ? null : resolveActiveSession(sessions, activeId, pendingCreatedSession, trashedSessions);
    // Desktop jump list (`␣⇥` previous, `␣O` Recent): every opened Session or
    // Draft, newest first, kept per device.
    const workspaceItemKey = draftRoute.id ? `draft:${draftRoute.id}` : active?.id ?? null;
    useEffect(() => {
        if (surface === "desktop" && workspaceItemKey) recordDesktopVisit(workspaceItemKey);
    }, [surface, workspaceItemKey]);
    const desktopVisits = useDesktopVisits();
    const desktopRecent = useMemo(
        () => surface === "desktop"
            ? desktopRecentItems(desktopVisits, workspaceItemKey, sessionsForView, draftLibrary.entries)
            : [],
        [desktopVisits, draftLibrary.entries, sessionsForView, surface, workspaceItemKey],
    );
    // Desktop tabs (desktop/workspaceTabs.ts): the item on screen always has
    // one, opened right of the tab it was reached from.
    const desktopTabState = useDesktopTabs();
    const tabOpenedFrom = useRef<string | null>(null);
    useEffect(() => {
        if (surface !== "desktop" || !workspaceItemKey) return;
        const from = tabOpenedFrom.current;
        tabOpenedFrom.current = workspaceItemKey;
        updateDesktopTabs((state) => withDesktopTab(state, workspaceItemKey, from));
    }, [surface, workspaceItemKey]);
    const workspaceItemExists = useCallback((key: string): boolean =>
        key.startsWith("draft:")
            ? draftLibrary.entries.some((entry) =>
                entry.id === key.slice("draft:".length) && entry.kind === "document" && !entry.deleted)
            : sessionsForView.some((session) => session.id === key),
    [draftLibrary.entries, sessionsForView]);
    useEffect(() => {
        // Only a complete, current picture may drop a tab as deleted.
        if (surface !== "desktop" || !sessionsLoaded || !draftLibrary.loaded || draftLibrary.error) return;
        updateDesktopTabs((state) =>
            pruneDesktopTabs(state, (key) => key === workspaceItemKey || workspaceItemExists(key)));
    }, [surface, sessionsLoaded, draftLibrary.loaded, draftLibrary.error, workspaceItemExists, workspaceItemKey]);
    const desktopTabItems = useMemo((): DesktopTabItem[] => {
        if (surface !== "desktop") return [];
        return desktopTabState.tabs.flatMap((tab): DesktopTabItem[] => {
            if (tab.key.startsWith("draft:")) {
                const draft = draftLibrary.entries.find((entry) => entry.id === tab.key.slice("draft:".length));
                if (!draft || draft.deleted) return [];
                return [{
                    ...tab,
                    kind: "draft",
                    title: draft.title || "Untitled draft",
                    leading: <DescriptionOutlined data-desktop-tab-icon />,
                }];
            }
            const session = sessionsForView.find((candidate) => candidate.id === tab.key);
            if (!session) return [];
            return [{
                ...tab,
                kind: "session",
                title: session.title.startsWith(`${session.provider} · `)
                    ? session.title.slice(session.provider.length + 3)
                    : session.title,
                leading: (
                    <>
                        <StatusDot status={session.status} backgroundTasks={session.background_tasks} providerUpdate={session.provider_update} resourceWait={session.resource_wait} />
                        <ProviderIcon
                            provider={session.provider}
                            providerVersion={session.provider_version}
                            providerDigest={session.provider_generation_digest}
                            fontSize="small"
                            data-desktop-tab-icon
                        />
                    </>
                ),
            }];
        });
    }, [desktopTabState.tabs, draftLibrary.entries, sessionsForView, surface]);
    const desktopVisitKeys = useMemo(() => desktopVisits.map((visit) => visit.key), [desktopVisits]);
    // The boot overlay is showing a picture of the last screen; hand over as
    // soon as the real one is on screen (docs/offline-first-sync.md §Boot
    // presentation). A cached tail counts: it is what the user came to read.
    const activePainted = useStoreSelector((snapshot) =>
        active === null || snapshot.hydrated.has(active.id) ||
        (snapshot.timelines.get(active.id)?.length ?? 0) > 0
    );
    useBootPresentation({
        painted: sessionsLoaded && activePainted,
        sessionId: active?.id ?? null,
        themeMode,
        busy: active?.status === "busy" || active?.status === "starting",
    });
    useEffect(() => {
        if (pendingCreatedSession && sessions.some((session) => session.id === pendingCreatedSession.id)) {
            setPendingCreatedSession(null);
        }
    }, [pendingCreatedSession, sessions]);
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
        if (sessionsSource !== "live") return;
        retainTranscriptViewportSessions(new Set(sessions.map((session) => session.id)));
    }, [sessions, sessionsSource]);

    useSessionFocusLifecycle({
        active,
        sessions,
        trashedSessions,
        sessionsSource,
        draftActive: draftRoute.active,
        restoredFocusRef,
        goneCheckedRef,
    });

    // A surface outside this app (the Mobile connection sheet) opens a session
    // through the same path as a sessions-list tap, so drawer settling and the
    // persisted active id stay owned here.
    useEffect(() => {
        const onPickRequest = (event: Event): void => {
            const id = pickSessionDetail(event);
            const created = (event as CustomEvent<{ session?: SessionMeta }>).detail?.session;
            if (created?.id === id) setPendingCreatedSession(created);
            if (id !== null) pick(id);
        };
        globalThis.addEventListener(PICK_SESSION_EVENT, onPickRequest);
        return () => globalThis.removeEventListener(PICK_SESSION_EVENT, onPickRequest);
    });

    function pick(id: string): void {
        if (draftRoute.id) {
            void draftBeforeLeave.current().then(() => selectWorkspaceItem(id))
                .catch((error: Error) => documentNotice(error.message));
        } else selectWorkspaceItem(id);
    }
    function selectWorkspaceItem(id: string): void {
        globalThis.dispatchEvent(
            new CustomEvent("cowboy:transcript-save-viewport"),
        );
        // Selection is navigation, so acknowledge it in the same input frame.
        // The target shell (session identity, status and composer) can render
        // immediately while its independently-hydrated transcript shows its own
        // structural loading state. Delaying this until the drawer's settle
        // callback made a successful tap look ignored, then changed everything
        // at once after the motion had finished.
        if (id.startsWith("draft:")) openDrafts(id.slice(6));
        else { openSession(id); setActiveId(id); }
        if (mobile) {
            // Close before React can rebind the drawer. A session switch used
            // to remount the controller and restore the still-open ref.
            drawerOpenRef.current = false;
            onMobileDrawerOpenChange?.(false);
            if (settleMobileDrawerRef.current) {
                settleMobileDrawerRef.current(false);
            } else {
                setDrawerOpen(false);
            }
        } else {
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
        // Overshooting the minimum folds Sessions away on release; the drag
        // previews that by dimming the rail. Its width stays what it was.
        let collapse = false;
        const onMove = (ev: PointerEvent): void => {
            const raw = startWidth + (ev.clientX - startX);
            const next = desktopWorkspace !== null && dragCollapses(raw, SIDEBAR_MIN);
            if (next !== collapse) {
                collapse = next;
                setSessionsCollapseIntent(next);
            }
            setSidebarWidth(clampSidebarWidth(collapse ? startWidth : raw));
        };
        const onUp = (): void => {
            el.releasePointerCapture(e.pointerId);
            el.removeEventListener("pointermove", onMove);
            el.removeEventListener("pointerup", onUp);
            setResizing(false);
            setSessionsCollapseIntent(false);
            sidebarWidthStore.set(widthRef.current);
            if (collapse) desktopWorkspace?.togglePane("sessions");
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
        // Measure the rendered geometry: the stored width may exceed what the
        // Conversation floor lets Prompt occupy. Overshooting either pane's
        // floor folds that pane away on release and keeps the stored width.
        const renderedPrompt = el.previousElementSibling?.getBoundingClientRect().width ?? startWidth;
        const containerWidth = el.parentElement?.getBoundingClientRect().width ?? 0;
        let collapse: "prompt" | "conversation" | null = null;
        const ratioLayout = sessionsCollapsed && containerWidth > 0;
        const startRatio = promptRatioRef.current;
        const onMove = (ev: PointerEvent): void => {
            const dx = ev.clientX - startX;
            const rawPrompt = renderedPrompt + dx;
            const next = desktopWorkspace === null
                ? null
                : dragCollapses(rawPrompt, DESKTOP_PROMPT_MIN)
                ? "prompt"
                : dragCollapses(containerWidth - rawPrompt, DESKTOP_CONVERSATION_MIN)
                ? "conversation"
                : null;
            if (next !== collapse) {
                collapse = next;
                setWorkCollapseIntent(next);
            }
            if (ratioLayout) {
                setPromptRatio(collapse
                    ? startRatio
                    : clampPromptRatio((renderedPrompt + dx) / containerWidth));
            } else {
                setColWidth(clampComposerColWidth(collapse ? startWidth : startWidth + dx));
            }
        };
        const onUp = (): void => {
            el.releasePointerCapture(e.pointerId);
            el.removeEventListener("pointermove", onMove);
            el.removeEventListener("pointerup", onUp);
            setColResizing(false);
            setWorkCollapseIntent(null);
            if (ratioLayout) collapsedSessionsPromptRatioStore.set(promptRatioRef.current);
            else composerColWidthStore.set(colWidthRef.current);
            if (collapse) desktopWorkspace?.togglePane(collapse);
        };
        el.addEventListener("pointermove", onMove);
        el.addEventListener("pointerup", onUp);
    }

    const list = (
        <SessionList
            sessions={sessionsForView}
            drafts={draftLibrary.entries}
            onDraftAction={(action) => flushSync(() => setDraftAction(action))}
            onDraftCopy={(id, session) => {
                void (async () => {
                    if (draftRoute.id === id) await draftBeforeLeave.current();
                    await copyDraftToSession(id, session.id, session.title);
                })().catch((error: Error) => documentNotice(error.message));
            }}
            activeId={draftRoute.id ? `draft:${draftRoute.id}` : active?.id ?? null}
            onPick={pick}
            onNew={openNewSession}
            onNewInFolder={(folder): void => {
                newSessionFolderRef.current = folder;
                openNewSession();
            }}
            allowNewSession
            desktopHeaderActions={surface === "desktop" && !sessionsInDrawer && !sessionsCollapsed
                ? sessionsHeaderActions
                : null}
            onClose={mobile
                ? (): void => settleMobileDrawerRef.current?.(false)
                : undefined}
            onOpenSettings={mobile
                ? (): void => openAppSettings({ section: "agent" })
                : undefined}
            onOpenAbout={mobile
                ? (): void => openAppSettings({ tab: "info" })
                : undefined}
            onRequestDelete={requestDelete}
            onRequestInfo={(s): void => setPendingInfo(s)}
            onRequestReload={(s): void => setPendingReload(s)}
            onRequestRename={(s): void => {
                // Mount the real Title input while the rename tap still owns
                // iOS user activation. RenameSessionShell's layout effect then
                // focuses it before flushSync returns, so no hidden-input handoff
                // or post-gesture focus is needed.
                flushSync(() => setPendingRename(s));
            }}
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
    // The docked sidebar's two bars, VS Code with its activity bar on top:
    // the activity bar (views left: Sessions, the shown one, then Extensions;
    // the window's Settings and the pane fold right) aligned with the work
    // panes' top bar, then the shown view's title bar (its name and its own
    // actions, Create and fold). Every entry is the pane-fold capsule: glyph
    // plus quiet leader slot, sized in units that follow the global font size.
    const sidebarHeader = (
        <Box data-desktop-pane-header sx={{ flexShrink: 0 }}>
        <Toolbar
            data-desktop-sessions-activity-bar
            variant="dense"
            sx={{
                ...SESSIONS_ACTIVITY_SLOT_FIT_SX,
                borderBottom: 1,
                borderColor: "divider",
                gap: desktopSize(6),
                px: desktopSize(10),
                // Match the right pane's AppBar Toolbar EXACTLY, so the sidebar
                // bar's bottom border lines up with the chat header's edge.
                minHeight: desktopSize(44),
                "@media (min-width: 600px)": { minHeight: desktopSize(44), px: desktopSize(10) },
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
            {surface === "desktop" ? (
                <Suspense fallback={null}>
                    <Box
                        data-desktop-sessions-activities
                        role="toolbar"
                        aria-label="Sessions views"
                        sx={{ display: "flex", alignItems: "center", gap: desktopSize(6), WebkitAppRegion: "no-drag" }}
                    >
                        <DesktopSidebarAction
                            data-desktop-sessions-view="sessions"
                            label="Sessions"
                            shortcut={DESKTOP_SHORTCUTS.focusSessions}
                            active
                            onClick={(): void => desktopWorkspace?.focusPane("sessions")}
                        >
                            <ViewListOutlined />
                        </DesktopSidebarAction>
                        {!sessionsCollapsed && (
                            <DesktopExtensionsActivity
                                sessionId={active?.id}
                                machineId={active?.machine_id}
                            />
                        )}
                    </Box>
                    <Box sx={{ flex: 1 }} />
                    {!sessionsCollapsed && (
                        <DesktopSettingsAction
                            onOpenSettings={(): void => openSettings("settings")}
                        />
                    )}
                    <DesktopSidebarAction
                        data-desktop-pane-collapse="sessions"
                        label="Collapse Sessions"
                        shortcut={DESKTOP_SHORTCUTS.toggleSessions}
                        onClick={(): void => desktopWorkspace?.togglePane("sessions")}
                    >
                        <KeyboardDoubleArrowLeft />
                    </DesktopSidebarAction>
                </Suspense>
            ) : (
                <Typography
                    variant="overline"
                    sx={{
                        color: "text.secondary",
                        fontWeight: 700,
                        letterSpacing: "0.09em",
                        lineHeight: 1,
                    }}
                >
                    Sessions
                </Typography>
            )}
        </Toolbar>
        {surface === "desktop" && (
            <Box
                data-desktop-sessions-view-title
                sx={{
                    minHeight: desktopSize(34),
                    px: desktopSize(10),
                    display: "flex",
                    alignItems: "center",
                    gap: desktopSize(6),
                }}
            >
                <Typography
                    variant="overline"
                    noWrap
                    sx={{
                        flex: "1 1 auto",
                        minWidth: 0,
                        color: "text.secondary",
                        fontSize: desktopSize(11),
                        fontWeight: 700,
                        letterSpacing: "0.09em",
                        lineHeight: 1,
                    }}
                >
                    Sessions
                </Typography>
                <Box
                    data-desktop-sessions-header-actions
                    role="toolbar"
                    aria-label="Sessions actions"
                    ref={setSessionsHeaderActions}
                    sx={{ display: "flex", alignItems: "center", gap: desktopSize(6), flexShrink: 0 }}
                />
            </Box>
        )}
        </Box>
    );

    return (
        <Box
            sx={{
                display: "flex",
                flexDirection: "column",
                height: "100%",
                width: "100%",
                // DetentSheet is an inline overlay. Let the browser's style
                // engine hide the underlying floating chrome from the sheet's
                // own data marker, instead of subscribing App to the global
                // sheet-open store and forcing a full App render on every open.
                "&:has([data-detent-sheet='true']) [data-detent-sheet-chrome='true']": {
                    opacity: 0,
                    pointerEvents: "none",
                },
                ...(surface === "desktop" && {
                    // The installed-app title bar and the top bar share one
                    // surface color; a hairline keeps them two layers.
                    borderTop: 1,
                    borderColor: "divider",
                    "& [data-desktop-region]": {
                        position: "relative",
                        outline: "none",
                        transition: "background-color 120ms ease, border-color 120ms ease, box-shadow 120ms ease",
                    },
                    "& [data-desktop-region][data-desktop-focused='true']:not([data-desktop-region='prompt.calls']):not([data-desktop-region='prompt.plan']):not([data-desktop-region='prompt.queued']):not([data-desktop-region='prompt.draft']):not([data-desktop-region='prompt.composer']):not([data-desktop-region='topbar.controls']):not([data-desktop-region='conversation.transcript']):not([data-desktop-region='sessions.list'])": {
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
                    // The Calls and Plan region wrappers have no visual
                    // geometry of their own. Recolor the real dock surface
                    // (the Calls and Plan disclosure rails), exactly
                    // like Queue/Draft, instead of drawing a square ring
                    // around its margin box.
                    "& [data-desktop-region='prompt.calls'][data-desktop-focused='true'] > [data-desktop-calls-surface], & [data-desktop-region='prompt.plan'][data-desktop-focused='true'] > [data-desktop-plan-surface]": {
                        borderColor: desktopFocusBoundary,
                        bgcolor: desktopFocusFill,
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
                    // Top Bar is a workspace region, exclusive with Sessions
                    // and Prompt. Paint the strip itself with the shared focus
                    // fill and a primary underline. Do not set `color` on the
                    // Toolbar: every color="inherit" control would then look
                    // selected. The keyboard cursor still belongs to the
                    // focused control; programmatic focus() is not reliably
                    // :focus-visible on macOS, so combine region ownership
                    // with real DOM :focus just as Sessions does below.
                    "& [data-desktop-region='topbar.controls'][data-desktop-focused='true']": {
                        bgcolor: desktopFocusFill,
                        boxShadow: (t) =>
                            `inset 0 -2px 0 0 ${t.palette.primary.main}`,
                    },
                    "& [data-desktop-region='topbar.controls'][data-desktop-focused='true'] [data-desktop-item]:focus": {
                        outline: "none",
                        borderColor: "primary.main",
                        bgcolor: (t) => alpha(t.palette.primary.main, 0.105),
                        boxShadow: (t) =>
                            `0 0 0 2px ${alpha(t.palette.primary.main, 0.16)}`,
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
                        bgcolor: (t) => alpha(t.palette.primary.main, 0.04),
                        // Inset: Sessions rows are flush with the rail, whose
                        // scroller would clip an outer ring.
                        boxShadow: (t) =>
                            `inset 0 0 0 1px ${alpha(t.palette.primary.main, 0.5)}`,
                        "& .cowboy-session-grip": {
                            color: "primary.main",
                        },
                        "& .cowboy-session-shortcut": {
                            opacity: "1 !important",
                        },
                    },
                    "& [data-desktop-region='sessions.list'][data-desktop-focused='true'] [data-desktop-item][data-desktop-current='true']:focus": {
                        // The open session is focused after every click, so
                        // a ring here would box it permanently. The cursor
                        // resting on it deepens the fill instead.
                        bgcolor: (t) => alpha(t.palette.primary.main, CURRENT_ROW_FILL + 0.06),
                        boxShadow: "none",
                    },
                    "& [data-desktop-region='sessions.list'][data-desktop-pinned='true'] [data-desktop-item][data-desktop-pin-active='true']:focus": {
                        bgcolor: (t) => alpha(t.palette.primary.main, 0.105),
                        boxShadow: (t) =>
                            `inset 0 0 0 2px ${alpha(t.palette.primary.main, 0.48)}, 0 3px 14px ${alpha(t.palette.primary.main, 0.09)}`,
                    },
                }),
            }}
        >
            <PriorSendDecisionSheet />
            {/* The connection state is shared; so is the update policy. Both
                surfaces download the deployed build at once, fill the bar with
                it, and reload on their own after a countdown once the user is
                idle. Device settings select automatic countdown or manual
                application; zero explicitly applies immediately after download. */}
            {surface === "desktop" && (
                <>
                    {/* Connectivity lives in the status line; the banner keeps
                        only the update decision and never reloads mid-work. */}
                    <ConnectionBanner
                        store={controlPlaneConnection}
                        automatic={clientUpdateSettings.mode === "automatic"}
                        countdownSecs={clientUpdateSettings.countdownSecs}
                        kinds={["update"]}
                        canApplyUpdate={canApplyUpdateNow}
                        beforeReload={markDesktopUpdateSwapping}
                        shortcut={desktopShortcutCaps("Mod+R").join(" ")}
                    />
                    <NativeReleaseUpdatePrompt
                        appId="top.thundersparrow.cowboy"
                        manifestUrl="/native-release.json"
                    />
                </>
            )}
            <Box
                ref={mobileShellRef}
                data-mobile-drawer-presented={mobile && drawerOpen ? "true" : undefined}
                sx={{
                    display: "flex",
                    flex: 1,
                    minHeight: 0,
                    width: "100%",
                    position: "relative",
                    // Desktop uses `clip`: unlike `hidden` it is never a scroll
                    // container, so focus/scrollIntoView cannot shift the whole
                    // workspace (Sessions rail included) sideways.
                    overflow: mobile ? "hidden" : "clip",
                    // Store notifications are already held during a drawer
                    // gesture. Collapse per-row paint tiles at rest so the
                    // first tracking frame only writes transform. Overflow
                    // freeze stays on the claimed-swipe selector — a settled
                    // open/presented flatten leaks `overflow:hidden` onto the
                    // transcript if close settle is cancelled by a later touch.
                    ...mobilePeekRestLayerSx,
                    ...mobilePresentationMovingRootSx(
                        "data-mobile-drawer-moving",
                    ),
                    ...mobileDrawerRailHitSx,
                }}
            >
            {surface === "desktop" && (
                <Suspense fallback={null}>
                    <DesktopCommandHost
                        onNewSession={openNewSession}
                        onOpenSettings={(): void => openSettings("settings")}
                        recent={desktopRecent}
                        onOpenRecent={pick}
                        onRenameSession={active && !draftRoute.id
                            ? (): void => setPendingRename(active)
                            : undefined}
                        draftOpen={!!draftRoute.id}
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
                        isolation: "isolate",
                        transform: "translate3d(0, 0, 0)",
                    }}
                >
                    <Stack
                        role="navigation"
                        aria-label="Sessions"
                        sx={{
                            width: MOBILE_SESSION_DRAWER_WIDTH,
                            height: "100%",
                            minWidth: 0,
                            overflow: "hidden",
                            bgcolor: "background.default",
                            pointerEvents: "auto",
                            position: "relative", ...readerTopEdge(theme, "var(--cowboy-system-top-clearance, 0px)"),
                            pl: "env(safe-area-inset-left, 0px)",
                            "@media (min-width: 768px)": {
                                width:
                                    "var(--mobile-drawer-width, min(52%, 440px))",
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
                        top: 0,
                        bottom: 0,
                        left: 0,
                        width: 28,
                        bgcolor: "background.default",
                        // The mask used to carry a projected shadow. Obsidian
                        // keeps a hard paper|page join, so the strip is only
                        // paint insurance in the peek color.
                        boxShadow: drawerOpen
                            ? mobileSpatialDrawerShadow("left")
                            : "none",
                        pointerEvents: "none",
                        backfaceVisibility: "hidden",
                        overflow: "visible",
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
                <>
                {sessionsCollapsed && (
                    <Suspense fallback={null}>
                        <DesktopSessionsRail
                            groups={collapsedRail.groups}
                            activeId={draftRoute.id ? `draft:${draftRoute.id}` : active?.id ?? null}
                            allowNewSession
                            onPick={pick}
                            onNew={openNewSession}
                            footer={
                                <DesktopRailGlobalActions
                                    sessionId={active?.id}
                                    machineId={active?.machine_id}
                                    onOpenSettings={(): void => openSettings("settings")}
                                />
                            }
                            renderStatus={(s): React.ReactNode => (
                                <StatusDot status={s.status} backgroundTasks={s.background_tasks} providerUpdate={s.provider_update} resourceWait={s.resource_wait} />
                            )}
                        />
                    </Suspense>
                )}
                <Stack
                    data-desktop-pane="sessions"
                    data-desktop-pane-collapsed={sessionsCollapsed ? "true" : undefined}
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
                        // Collapsed Sessions stays mounted: its list still owns
                        // the ␣␣ session labels and folder state.
                        display: sessionsCollapsed ? "none" : "flex",
                        opacity: sessionsCollapseIntent ? 0.38 : 1,
                        transition: "opacity 120ms ease",
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
                    <HintTooltip
                        title="Resize layout"
                        shortcut={DESKTOP_SHORTCUTS.resize}
                        followCursor
                        // A pointer hint: selecting the splitter by keyboard shows
                        // its own Resize hint instead.
                        disableFocusListener
                        placement="right"
                        suppressed={resizing ||
                            desktopWorkspace?.selectedSplitter === "sessions-prompt"}
                    >
                        <Box
                            role="separator"
                            aria-orientation="vertical"
                            aria-label="Resize sidebar"
                            aria-valuemin={SIDEBAR_MIN}
                            aria-valuemax={SIDEBAR_MAX}
                            aria-valuenow={Math.round(sidebarWidth)}
                            data-desktop-splitter="sessions-prompt"
                            data-desktop-splitter-selected={
                                desktopWorkspace?.selectedSplitter === "sessions-prompt"
                                    ? "true"
                                    : undefined
                            }
                            tabIndex={-1}
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
                                    bgcolor: resizing ||
                                            desktopWorkspace?.selectedSplitter ===
                                                "sessions-prompt"
                                        ? "primary.main"
                                        : "divider",
                                    transition: "background-color 120ms",
                                },
                                "&:hover::after": {
                                    bgcolor: "primary.main",
                                },
                                "&:focus": { outline: "none" },
                            }}
                        >
                            {desktopWorkspace?.selectedSplitter ===
                                    "sessions-prompt" && (
                                <Suspense fallback={null}>
                                    <DesktopSplitterHint />
                                </Suspense>
                            )}
                        </Box>
                    </HintTooltip>
                </Stack>
                </>
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
                    // Mobile paint lives on the sliding page. An opaque
                    // column here covers the Sessions rail when the page
                    // translates.
                    bgcolor: mobile ? "transparent" : "background.default",
                    // The sliding children own hits. This wrapper stays
                    // full-width; leaving it clickable would swallow
                    // Sessions-row taps through the revealed rail.
                    pointerEvents: mobile ? "none" : undefined,
                    // Lift the whole column off the on-screen keyboard + its
                    // iOS-native accessory bar: this padding (the keyboard's
                    // overlap, published by useKeyboardInset) reserves space at
                    // the bottom, so the flex:1 transcript shrinks and the bottom
                    // group (composer, or the navbar in bottom mode) rises clear
                    // of the keyboard. 0 when no keyboard.
                    pb: "var(--kb-inset, 0px)",
                    // Keyboard up: temporarily tuck the session nav so the
                    // compact composer sits on the keyboard. The two-track
                    // format dock stays edit-only; this only hides chrome.
                    "&:has([data-mobile-focus-composer='true'][data-mobile-keyboard-open='true']:focus-within) [data-mobile-session-nav='true']": {
                        minHeight: 0,
                        maxHeight: 0,
                        opacity: 0,
                        transform: "translateY(10px)",
                        pointerEvents: "none",
                        overflow: "hidden",
                        paddingTop: 0,
                        paddingBottom: 0,
                        transition:
                            `max-height ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, min-height ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, opacity 90ms ease, transform ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, padding ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}`,
                    },
                    "&:has([data-mobile-focus-composer='true'][data-mobile-keyboard-open='true']:focus-within) [data-mobile-composer-shell-material='true']": {
                        opacity: "0 !important",
                    },
                }}
            >
            <Box
                ref={mobile ? mobileLayerRef : undefined}
                sx={mobile
                    ? {
                        // In-flow fill, not position:absolute;inset:0. iOS
                        // pins bottom chrome of a viewport-sized absolutely
                        // positioned layer while its transform is changing.
                        flex: 1,
                        minHeight: 0,
                        width: "100%",
                        display: "flex",
                        flexDirection: "column",
                        position: "relative",
                        isolation: "isolate",
                        bgcolor: "transparent",
                        pointerEvents: "none",
                        backfaceVisibility: "hidden",
                        transformOrigin: "left center",
                    }
                    : { display: "contents" }}
            >
                {mobile && (
                    <Box
                        ref={mobileDrawerDimRef}
                        data-mobile-drawer-dim="left"
                        aria-hidden
                        sx={{
                            position: "absolute",
                            inset: 0,
                            zIndex: (t) => t.zIndex.modal - 1,
                            // Paint only. A post-settle left/transform rewrite
                            // restretches the gradient and makes the veil jump.
                            bgcolor: "transparent",
                            backgroundImage: (t) =>
                                t.palette.mode === "dark"
                                    ? "linear-gradient(to right, rgba(0,0,0,0.46), rgba(0,0,0,0.20) 42%, rgba(0,0,0,0.05))"
                                    : "linear-gradient(to right, rgba(0,0,0,0.24), rgba(0,0,0,0.10) 46%, rgba(0,0,0,0.03))",
                            opacity: 0,
                            pointerEvents: "none",
                        }}
                    />
                )}
                {/* A mobile Draft is a focus-on-writing page (Obsidian): its
                    own floating top controls replace the bottom nav and the
                    glass slab behind it. */}
                {mobile && !splitActive && !draftRoute.id && (
                    <Box
                        ref={mobileFrostFollowRef}
                        aria-hidden
                        data-mobile-composer-shell-material="true"
                        data-mobile-backdrop-chrome="true"
                        data-mobile-drawer-follow="true"
                        sx={(t) => ({
                            position: "absolute",
                            left: 0,
                            right: 0,
                            bottom: 0,
                            height: floatingPanelHeight,
                            zIndex: 1,
                            pointerEvents: "none",
                            // Resting glass. This follower already has its
                            // own translate3d; keep the filter during the
                            // swipe so prepare does not rebuild the layer.
                            ...frostedChrome(t),
                        })}
                    />
                )}
                {mobile && (
                    <Box
                        data-mobile-drawer-close="left"
                        role="button"
                        tabIndex={drawerOpen ? 0 : -1}
                        aria-label="Close sessions"
                        aria-hidden={!drawerOpen}
                        onPointerDown={closeSessionsPeek.onPointerDown}
                        onPointerMove={closeSessionsPeek.onPointerMove}
                        onPointerUp={closeSessionsPeek.onPointerUp}
                        onPointerCancel={closeSessionsPeek.onPointerCancel}
                        onClick={closeSessionsPeek.onClick}
                        onKeyDown={(event): void => {
                            if (
                                !isImeKeyEvent(event.nativeEvent) &&
                                (event.key === "Enter" || event.key === " ")
                            ) {
                                event.preventDefault();
                                settleMobileDrawerRef.current?.(false);
                            }
                        }}
                        sx={{
                            position: "absolute",
                            top: 0,
                            right: 0,
                            bottom: 0,
                            left: 0,
                            zIndex: (t) => t.zIndex.modal - 1,
                            pointerEvents: drawerOpen ? "auto" : "none",
                            cursor: drawerOpen ? "pointer" : "default",
                        }}
                    />
                )}
                {/* Mobile Transcript owns its paint-only safe-area gradient;
                    keep this follower unpainted there to avoid doubling the veil.
                    Top mode (desktop): a solid, full navbar-height
                    slab behind the transparent top AppBar, so Desktop keeps its
                    dense, stable chrome. Height tracks the measured
                    --navbar-h. pointer-events:none so taps reach the (lifted) navbar.
                    DROPPED in split mode: nothing scrolls under the AppBar (the transcript
                    is an in-flow column below it), so the bar just sits on the app surface. */}
                {!splitActive && (
                <Box
                    ref={mobile && navbarAtBottom ? mobileStatusChromeFollowRef : undefined}
                    aria-hidden
                    data-detent-sheet-chrome="true"
                    data-mobile-backdrop-chrome={mobile && navbarAtBottom ? "true" : undefined}
                    data-mobile-drawer-follow={mobile && navbarAtBottom ? "true" : undefined}
                    data-mobile-status-strip-material={mobile && navbarAtBottom ? "true" : undefined}
                    sx={(t) => ({
                        position: "absolute",
                        top: 0,
                        left: 0,
                        right: 0,
                        height: navbarAtBottom
                            ? "var(--cowboy-mobile-status-material-height)"
                            : "var(--navbar-h, 0px)",
                        // Behind the AppBar content (zIndex 2) but above the transcript (0).
                        // Bottom mode's strip sits alone at the top, so keep it high.
                        zIndex: navbarAtBottom ? ((t) => t.zIndex.appBar) : 1,
                        pointerEvents: "none",
                        // Hide under an open cover sheet — else this frosted strip
                        // bleeds through as a bright band under the status bar.
                        transition: "opacity 200ms ease",
                        ...(navbarAtBottom
                            ? (mobile ? {} : frostedStatusChrome(t))
                            : { backgroundColor: t.palette.background.default }),
                        // Desktop: a hairline delineates the navbar. In LIGHT mode the
                        // old `0 1px 24px` down-shadow smeared a gray cloud across the
                        // lavender (a black shadow on a light tint always reads gray) —
                        // dropped it; the hairline + the frosted blur are enough. Dark
                        // mode keeps a soft shadow (it reads as depth there, not gray).
                        ...(!navbarAtBottom && {
                            borderBottom: 1,
                            borderColor: "divider",
                            boxShadow: t.palette.mode === "dark"
                                ? "0 1px 24px rgba(0,0,0,0.5)"
                                : "none",
                        }),
                    })}
                />
                )}
                {/* ONE frosted-glass slab behind BOTH the composer and the navbar,
                    so they read as a single piece of glass — not two stacked panes
                    with a seam. Its height is exactly the measured floating stack,
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
                {!splitActive && !mobile && (
                <Box
                    aria-hidden
                    data-mobile-composer-shell-material="true"
                    data-detent-sheet-chrome="true"
                    sx={{
                            position: "absolute",
                            left: 0,
                            right: 0,
                            bottom: "var(--kb-inset, 0px)",
                            height: floatingPanelHeight,
                            zIndex: 1,
                            pointerEvents: "none",
                            transition: "opacity 200ms ease",
                            bgcolor: "transparent",
                            backdropFilter:
                                "blur(40px) saturate(180%) brightness(1.06)",
                            WebkitBackdropFilter:
                                "blur(40px) saturate(180%) brightness(1.06)",
                    }}
                />
                )}
                {!splitActive && !mobile && (
                <Box
                    aria-hidden
                    data-mobile-composer-shell-material="true"
                    data-detent-sheet-chrome="true"
                    sx={{
                        position: "absolute",
                        left: 0,
                        right: 0,
                        bottom: `calc(var(--kb-inset, 0px) + ${floatingPanelHeight})`,
                        height: "1px",
                        zIndex: 1,
                        pointerEvents: "none",
                        transition: "opacity 200ms ease",
                        bgcolor: "divider",
                        boxShadow: transcriptScrollable
                            ? (t) =>
                                `0 -1px 24px ${t.palette.mode === "dark" ? "rgba(0,0,0,0.5)" : "rgba(0,0,0,0.07)"}`
                            : "none",
                    }}
                />
                )}
                {surface === "desktop" && (
                    <Suspense fallback={null}>
                        <DesktopTabStrip
                            items={desktopTabItems}
                            currentKey={workspaceItemKey}
                            visits={desktopVisitKeys}
                            exists={workspaceItemExists}
                            onOpen={pick}
                            onRename={(key): void => {
                                const session = sessions.find((candidate) => candidate.id === key);
                                if (session) setPendingRename(session);
                            }}
                            // The window's top row beside the Sessions sidebar:
                            // installed-PWA window controls overlay its left
                            // whenever the sidebar is not next to it.
                            sx={{
                                ...(sessionsInDrawer && {
                                    "@media (display-mode: window-controls-overlay)": {
                                        pl: "calc(env(titlebar-area-x, 0px) + 12px)",
                                    },
                                }),
                                ...(sessionsCollapsed && {
                                    "@media (display-mode: window-controls-overlay)": {
                                        pl: "calc(max(0px, env(titlebar-area-x, 0px) - 56px) + 12px)",
                                    },
                                }),
                            }}
                        />
                    </Suspense>
                )}
                <AppBar
                    data-mobile-session-nav={navbarAtBottom ? "true" : undefined}
                    data-mobile-drawer-follow={mobile && navbarAtBottom ? "true" : undefined}
                    data-detent-sheet-chrome="true"
                    ref={attachAppBar}
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
                        pointerEvents: mobile ? "auto" : undefined,
                        flexShrink: mobile && navbarAtBottom ? 0 : undefined,
                        zIndex: mobile ? 3 : 2,
                        // DetentSheet is intentionally inline, so this AppBar's
                        // stacking context can otherwise sit above a sheet's
                        // bottom action island and receive the tap instead.
                        color: "text.primary",
                        maxHeight: navbarAtBottom ? 96 : undefined,
                        // Do not transition `transform` here. The Sessions
                        // drawer writes translate3d onto this bar every
                        // touchmove; a 240ms transform transition makes the
                        // bottom chrome trail the peek by a half-beat.
                        transition:
                            `max-height ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, min-height ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, opacity 110ms ease 70ms, padding ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}`,
                        "@media (prefers-reduced-motion: reduce)": {
                            transition: "none",
                        },
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
                        pt: navbarAtBottom ? 0 : "var(--cowboy-system-top-clearance)",
                        // Bottom gap = the home-indicator zone. The FULL inset (~34px)
                        // read as a big, lopsided gap vs the slim composer→navbar gap
                        // above; shave it so the bar sits closer to the edge while the
                        // 44px buttons still clear the indicator (an earlier `inset−20`
                        // ≈14px dropped them ONTO it — keep ≥ ~20px).
                        pb: navbarAtBottom ? "max(calc(env(safe-area-inset-bottom) - 18px), 12px)" : 0,
                        pl: navbarAtBottom ? "env(safe-area-inset-left, 0px)" : 0,
                        pr: navbarAtBottom ? "env(safe-area-inset-right, 0px)" : 0,
                        ...(mobile && draftRoute.id && { display: "none" }),
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
                            minHeight: desktopSize(44),
                            "@media (min-width: 600px)": { minHeight: desktopSize(44) },
                            // Narrow installed PWA: the sidebar has collapsed to a
                            // drawer, so this bar is full-width and the macOS window
                            // controls now overlay ITS left (the hamburger). Inset
                            // past them only in that mode — when the sidebar is
                            // visible the controls sit over it instead (see
                            // sidebarHeader), and this bar must stay flush.
                            // (On Desktop the tab bar above is that top row.)
                            ...(sessionsInDrawer && surface !== "desktop" && {
                                "@media (display-mode: window-controls-overlay)": {
                                    pl: "calc(env(titlebar-area-x, 0px) + 12px)",
                                },
                            }),
                            // The collapsed Sessions rail is narrower than the
                            // window controls, so they overhang this bar too.
                            ...(sessionsCollapsed && surface !== "desktop" && {
                                "@media (display-mode: window-controls-overlay)": {
                                    pl: "calc(max(0px, env(titlebar-area-x, 0px) - 56px) + 12px)",
                                },
                            }),
                        }}
                    >
                        {/* Whenever the Sessions rail is hidden, its drawer toggle
                            leads the bar. This does not change the product surface. */}
                        {sessionsInDrawer && (
                            // No `edge="start"`: its negative left margin pulled the
                            // hamburger tight to the screen edge, while Code (no
                            // edge="end") sits at the Toolbar's normal gutter — so the
                            // bar read lopsided. Dropping it gives the hamburger the
                            // same gutter, symmetric with the Code control on the right.
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
                                    "&[data-touch-activated='true']:hover, &[data-touch-activated='true'].Mui-focusVisible": {
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
                                data-desktop-active-session={
                                    surface === "desktop" ? "true" : undefined
                                }
                                direction="row"
                                alignItems="center"
                                spacing={0.75}
                                sx={{
                                    minWidth: 0,
                                    // A long title must end in an ellipsis inside
                                    // this cap, never run under the controls.
                                    overflow: "hidden",
                                    // Desktop gives the title only the width its
                                    // text needs (up to the cap). The old 280px
                                    // flex basis left a large dead zone after a
                                    // short title while squeezing the controls.
                                    flex: surface === "desktop" ? "0 1 auto" : 1,
                                    maxWidth: surface === "desktop" ? 320 : "none",
                                    ...(surface === "desktop" && sessionsInDrawer && !drawerOpen && {
                                        // When the Sessions rail is collapsed,
                                        // this is the only visible session tab.
                                        // Transfer the selected material here;
                                        // the drawer row owns it again when open.
                                        minHeight: 34,
                                        px: 0.75,
                                        border: 1,
                                        borderColor: (t) => alpha(t.palette.primary.main, 0.38),
                                        borderRadius: 1.5,
                                        bgcolor: (t) => alpha(t.palette.primary.main, 0.13),
                                        boxShadow: (t) =>
                                            `inset 0 0 0 1px ${alpha(t.palette.primary.main, 0.08)}`,
                                    }),
                                }}
                            >
                                <StatusDot status={active.status} backgroundTasks={active.background_tasks} providerUpdate={active.provider_update} resourceWait={active.resource_wait} />
                                <ProviderIcon
                                    provider={active.provider}
                                    providerVersion={active.provider_version}
                                    providerDigest={active.provider_generation_digest}
                                    fontSize="small"
                                    sx={{ flexShrink: 0 }}
                                />
                                <Tooltip title={active.title} enterDelay={400}>
                                    <Typography
                                        variant="subtitle1"
                                        noWrap
                                        sx={{ minWidth: 0, flex: "0 1 auto" }}
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
                        ) : null}
                        {surface === "desktop" ? (
                            // Session controls may fill leftover width; an empty
                            // toolbar does not reserve space without a session.
                            <Box
                                // The width the top-bar strip must fit inside.
                                // DesktopTopBarControls observes this element to
                                // pick its density; scrolling is now only the
                                // last resort, not the first answer to a narrow
                                // pane (desktop/topBarDensity.ts).
                                data-desktop-topbar-scroller
                                sx={{
                                    flex: active ? 1 : "0 0 auto",
                                    ml: "auto",
                                    minWidth: 0,
                                    overflowX: active ? "auto" : "visible",
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
                                    sx={{
                                        width: "max-content",
                                        minWidth: active ? "100%" : "auto",
                                    }}
                                >
                                    {active && (
                                        <Suspense fallback={null}>
                                            <DesktopTopBarControls
                                                sessionId={active.id}
                                                status={active.status}
                                            />
                                        </Suspense>
                                    )}
                                    {active && (
                                        <Suspense fallback={null}>
                                            <DesktopRegionShortcut
                                                shortcut={DESKTOP_SHORTCUTS.focusTopbar}
                                                title="Focus Top Bar"
                                                singleKeycap={DESKTOP_SHORTCUTS.focusTopbar}
                                                sx={{ mx: 0.5 }}
                                            />
                                        </Suspense>
                                    )}
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
                                {draftRoute.id && <IconButton aria-label="Draft actions" onClick={() => {
                                    const draft = draftLibrary.entries.find((entry) => entry.id === draftRoute.id);
                                    if (draft) flushSync(() => setDraftAction({ draft, action: "menu" }));
                                }}><MoreVert /></IconButton>}
                                {!draftRoute.active && <HintTooltip title="Code"><IconButton
                                    data-mobile-open-code="true"
                                    onPointerDown={(event): void => {
                                        if (event.pointerType === "touch") {
                                            event.currentTarget.dataset.touchActivated = "true";
                                        } else if (event.pointerType === "mouse") {
                                            delete event.currentTarget.dataset.touchActivated;
                                        }
                                        // By click time iOS may already have moved
                                        // document.activeElement to this navbar
                                        // button. Release the Composer while the
                                        // originating gesture still owns it so
                                        // its :focus-within height cannot outlive
                                        // the software keyboard.
                                        releaseMobileComposerFocus();
                                        openCodeTap.onPointerDown(event);
                                    }}
                                    onPointerEnter={(event): void => {
                                        if (event.pointerType === "mouse") {
                                            delete event.currentTarget.dataset.touchActivated;
                                        }
                                    }}
                                    onKeyDown={(event): void => {
                                        delete event.currentTarget.dataset.touchActivated;
                                    }}
                                    onPointerMove={openCodeTap.onPointerMove}
                                    onPointerUp={openCodeTap.onPointerUp}
                                    onPointerCancel={openCodeTap.onPointerCancel}
                                    onClick={openCodeTap.onClick}
                                    aria-label="Open code"
                                    sx={{
                                        // iOS WebKit latches the synthetic hover
                                        // generated by a finger tap. Keep the
                                        // pressed frame, then return the control
                                        // to its neutral material after release.
                                        "&[data-touch-activated='true']:hover, &[data-touch-activated='true'].Mui-focusVisible": {
                                            bgcolor: "transparent",
                                        },
                                        "&[data-touch-activated='true']:active": {
                                            bgcolor: "action.selected",
                                        },
                                    }}
                                >
                                    <CodeIcon />
                                </IconButton></HintTooltip>}
                            </>
                        )}
                    </Toolbar>
                </AppBar>

                {draftRoute.id ? (
                    <Box ref={mobile ? mobilePageRef : undefined}
                        data-mobile-drawer-surface={mobile ? "true" : undefined}
                        sx={{ flex: 1, minHeight: 0, minWidth: 0, position: "relative", pointerEvents: "auto", pt: mobile && navbarAtBottom ? "var(--cowboy-system-top-clearance)" : 0 }}>
                        <WorkspaceDraftPane id={draftRoute.id} beforeLeave={draftBeforeLeave}
                            onAction={(action) => flushSync(() => setDraftAction(action))}
                            mobileNavigation={mobile
                                ? {
                                    onOpenSessions: () => settleMobileDrawerRef.current?.(true),
                                }
                                : undefined} />
                    </Box>
                ) : active ? (
                    splitActive ? (
                        // ===== Two-column split layout (desktop opt-in) =====
                        // A flex ROW below the in-flow AppBar (order 1): composer column
                        // (left, fixed width) | drag splitter | transcript (right, flex:1).
                        // The transcript is a normal in-flow column here — NOT the absolute
                        // overlay — so it reserves no floating-stack inset and the composer no longer
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
                                collapseIntent={workCollapseIntent}
                                promptRatio={sessionsCollapsed ? promptRatio : null}
                                conversationActivity={(
                                    <StatusDot
                                        status={active.status}
                                        backgroundTasks={active.background_tasks}
                                        providerUpdate={active.provider_update}
                                        resourceWait={active.resource_wait}
                                    />
                                )}
                                prompt={active.system ? (
                                    <Box sx={{ p: 1.5, textAlign: "center", fontSize: desktopSize(13), opacity: 0.6 }}>
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
                        {/* Desktop: transcript is the full-height background
                            (absolute) and the navbar + composer float over it.
                            Mobile: the same column is in-flow so the footer
                            translates with the page instead of pinning. */}
                        <Box
                            ref={mobile ? mobilePageRef : undefined}
                            data-mobile-drawer-surface={mobile ? "true" : undefined}
                            sx={mobile
                                ? {
                                    // Transcript-only sliding page. Composer and
                                    // navbar are siblings with their own
                                    // translate3d; iOS pins them if they live
                                    // inside this transformed box. Isolation +
                                    // overflow keep the peek one compositor
                                    // layer at rest, matching the Review page.
                                    flex: 1,
                                    minHeight: 0,
                                    minWidth: 0,
                                    display: "flex",
                                    flexDirection: "column",
                                    position: "relative",
                                    zIndex: 0,
                                    isolation: "isolate",
                                    overflow: "hidden",
                                    backfaceVisibility: "hidden",
                                    transformOrigin: "left center",
                                    bgcolor: "background.default",
                                    pointerEvents: "auto",
                                }
                                : {
                                    position: "absolute",
                                    inset: 0,
                                    zIndex: 0,
                                    display: "flex",
                                    flexDirection: "column",
                                }}
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
                                // The iPad PWA shelf: content rests below the frost
                                // and may pass beneath it after scrolling. The iPad
                                // floor lives in the CSS variable, not a second
                                // Transcript pad; an iPhone PWA has no strip at all.
                                topInset={navbarAtBottom
                                    ? "var(--cowboy-mobile-status-material-height)"
                                    : "var(--navbar-h, 0px)"}
                                // Mobile footer is in-flow, so the transcript
                                // already ends above it. Desktop still floats.
                                bottomInset={mobile
                                    ? "0px"
                                    : "var(--transcript-bottom-inset, 0px)"}
                            />
                        </Box>
                        {/* Desktop still floats the composer over an absolute
                            transcript, so a flex spacer keeps the bars at the
                            column edges. Mobile owns a real in-flow column. */}
                        {!mobile && (
                        <Box aria-hidden sx={{ order: navbarAtBottom ? 0 : 1, flex: 1 }} />
                        )}
                        {/* Composer order: mobile = 1 (above the bottom navbar); desktop =
                            2 (the very bottom, after the spacer). position:relative + zIndex
                            lifts the frosted composer above the absolute transcript layer. */}
                        <Box
                            ref={attachComposer}
                            data-mobile-session-footer={mobile ? "true" : undefined}
                            data-mobile-drawer-follow={mobile ? "true" : undefined}
                            sx={{
                                order: navbarAtBottom ? 1 : 2,
                                minWidth: 0,
                                width: "100%",
                                alignSelf: "stretch",
                                ...(mobile && {
                                    // The keyboard-resized column is the hard
                                    // height boundary. Let an overlong Composer
                                    // shrink inside it so the editor, rather than
                                    // the formatting/delivery rows, owns overflow.
                                    minHeight: 0,
                                    maxHeight: "100%",
                                    display: "flex",
                                    flexDirection: "column",
                                    bgcolor: "transparent",
                                    pointerEvents: "auto",
                                    // Explore's Page Dock is an editor launcher and
                                    // navigation aid, not part of the writing
                                    // surface. Remove it from the floating stack
                                    // while the primary editor owns the keyboard;
                                    // it returns after blur so the close-editor
                                    // action remains available.
                                    "&:has([data-mobile-primary-composer='true'][data-mobile-keyboard-open='true'] [data-mobile-editor-area]:focus-within) > [data-mobile-page-dock='true']": {
                                        display: "none",
                                    },
                                }),
                                // Lift the composer content above the frosted slab behind
                                // it (zIndex 1) in BOTH modes — the composer is transparent,
                                // the slab is the glass, the transcript scrolls under both.
                                position: "relative",
                                zIndex: 2,
                            }}
                        >
                            {mobile && !splitActive && (
                                    <Box
                                        aria-hidden
                                        data-mobile-composer-shell-material="true"
                                        sx={{
                                            position: "absolute",
                                            left: 0,
                                            right: 0,
                                            top: 0,
                                            height: "1px",
                                            zIndex: 0,
                                            pointerEvents: "none",
                                            bgcolor: "divider",
                                            boxShadow: transcriptScrollable
                                                ? (t) =>
                                                    `0 -1px 24px ${
                                                        t.palette.mode === "dark"
                                                            ? "rgba(0,0,0,0.5)"
                                                            : "rgba(0,0,0,0.07)"
                                                    }`
                                                : "none",
                                        }}
                                    />
                            )}
                            {active.system ? (
                                <Box sx={{ p: 1.5, textAlign: "center", fontSize: desktopSize(13), opacity: 0.6 }}>
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
                    <Box
                        ref={mobile ? mobilePageRef : undefined}
                        data-mobile-drawer-surface={mobile ? "true" : undefined}
                        sx={{
                            flex: 1,
                            position: "relative",
                            pointerEvents: mobile ? "auto" : undefined,
                        }}
                    >
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
                                ? <EmptyWorkspaceState onNewSession={openNewSession} />
                                : (
                                    // Until the WS snapshot lands, the list is unknown — show the
                                    // spinner, NOT the empty "No session selected." CTA, so a reload
                                    // never flashes a false "create your first session".
                                    <LoadingState />
                                )}
                        </Stack>
                    </Box>
                )}
            </Box>
            </Stack>
            </Box>

            {draftAction && <WorkspaceDraftActions key={draftAction.draft.id + draftAction.action}
                request={draftAction} onClose={() => setDraftAction(null)} beforeLeave={draftBeforeLeave}
                activeId={draftRoute.id} onRemoved={leaveDrafts} />}
            <CreateDialog
                onDraftCreated={(id) => pick(`draft:${id}`)}
                open={dialogOpen}
                initialFolder={newSessionFolderRef.current}
                onClose={(): void => {
                    newSessionFolderRef.current = null;
                    setDialogOpen(false);
                }}
                onCreated={(session): void => {
                    // The daemon returns a durable Starting session before its
                    // Machine workspace and worker are ready. Select it now so
                    // preparation belongs to the destination page, not to the
                    // creation sheet. Do not defer this state behind drawer
                    // animation: the id already names a real persisted session.
                    setPendingCreatedSession(session);
                    setActiveId(session.id);
                    // Creation already records placement on the Controller.
                    // Do not enqueue a second move to a directory which may
                    // have been removed while the workspace was preparing.
                    newSessionFolderRef.current = null;
                    if (mobile && settleMobileDrawerRef.current) {
                        settleMobileDrawerRef.current(false, 0);
                    } else {
                        setDrawerOpen(false);
                    }
                }}
            />
            <DeleteSessionShell
                session={pendingDelete}
                onClose={(): void => setPendingDelete(null)}
                onStopAndTrash={(): void => {
                    if (pendingDelete) movedToTrash(pendingDelete, pendingDelete.status === "busy" ? stopAndTrashSession(pendingDelete.id) : trashItems({ sessions: [pendingDelete.id] }));
                    setPendingDelete(null);
                }}
                onDeletePermanently={(): void => {
                    if (pendingDelete) void deleteSession(pendingDelete.id);
                    setPendingDelete(null);
                }}
            />
            {/* Folder sheets opened from inside the Sessions list mount here,
                beside the session shells: outside the Mobile drawer layer and
                inside the keyboard-resized app box (see SessionFolderUi). */}
            <div {...{ [SESSION_FOLDER_SHEET_HOST]: "" }} />
            {pendingRename && (
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
            )}
            <SessionInfoShell session={pendingInfo} onClose={(): void => setPendingInfo(null)} />
            <SessionReloadDialog
                session={pendingReload}
                onClose={(): void => setPendingReload(null)}
            />
            <ResourceLightbox />
            <SettingsController
                ref={settingsControllerRef}
                mobile={mobile}
                themeMode={themeMode}
                onSetThemeMode={onSetThemeMode}
            />
            <Snackbar
                open={errorOpen}
                // One duration for every notice (hover pauses it).
                autoHideDuration={UNDO_NOTICE_MS}
                onClose={(): void => setShownErrorSeq(lastError?.seq ?? 0)}
                anchorOrigin={{
                    vertical: "bottom",
                    horizontal: mobile ? "center" : "right",
                }}
                // Lift above the measured Composer stack on mobile so the toast
                // never covers the input or its action row.
                sx={{
                    bottom: {
                        xs: MOBILE_NOTICE_BOTTOM,
                        sm: 24,
                    },
                }}
            >
                <Alert
                    severity={lastError?.severity ?? "error"}
                    variant="filled"
                    action={providerAuthShortcut
                        ? (
                            <Button
                                color="inherit"
                                size="small"
                                onClick={(): void => {
                                    setShownErrorSeq(lastError?.seq ?? 0);
                                    openSettings("providers", undefined, {
                                        providerId: providerAuthShortcut.providerId,
                                        autoBeginAuthentication: true,
                                    });
                                }}
                            >
                                {providerAuthShortcut.actionLabel}
                            </Button>
                        )
                        : undefined}
                    onClose={(): void => setShownErrorSeq(lastError?.seq ?? 0)}
                    sx={{ maxWidth: 480 }}
                >
                    {providerAuthShortcut?.message ?? lastError?.message ?? ""}
                </Alert>
            </Snackbar>
        </Box>
    );
}

const SettingsController = memo(forwardRef<
    SettingsControllerHandle,
    {
        mobile: boolean;
        themeMode: ThemeMode;
        onSetThemeMode: (m: ThemeMode) => void;
    }
>(function SettingsController({ mobile, themeMode, onSetThemeMode }, ref) {
    const [open, setOpen] = useState(false);
    const [initialTab, setInitialTab] = useState<SettingsTab>("settings");
    const [initialSection, setInitialSection] = useState<SettingsProductFocus>(
        "agent",
    );
    const providerRequestSequence = useRef(0);
    const [initialProviderIntent, setInitialProviderIntent] = useState<
        ProviderSettingsIntent | undefined
    >(undefined);
    const openSettings = useCallback((
        tab: SettingsTab,
        section: SettingsProductFocus = "agent",
        providerTarget?: ProviderSettingsTarget,
    ): void => {
        // A full-cover Mobile sheet ends the current Composer focus session.
        // This stays inside the isolated controller so the focus boundary does
        // not also schedule a render of the transcript/composer owner.
        if (mobile) releaseMobileComposerFocus();
        setInitialTab(tab);
        setInitialSection(section);
        if (providerTarget) {
            providerRequestSequence.current += 1;
            setInitialProviderIntent({
                ...providerTarget,
                requestId: providerRequestSequence.current,
            });
        } else {
            setInitialProviderIntent(undefined);
        }
        setOpen(true);
    }, [mobile]);
    useEffect(() => {
        const onOpen = (event: Event): void => {
            const detail = appSettingsFromEvent(event);
            openSettings(detail.tab ?? "settings", detail.section ?? "agent");
        };
        globalThis.addEventListener(OPEN_APP_SETTINGS_EVENT, onOpen);
        return () => {
            globalThis.removeEventListener(OPEN_APP_SETTINGS_EVENT, onOpen);
        };
    }, [openSettings]);
    useImperativeHandle(ref, () => ({ open: openSettings }), [openSettings]);
    return (
        <SettingsShell
            open={open}
            onClose={(): void => setOpen(false)}
            initialTab={initialTab}
            initialSection={initialSection}
            initialProviderIntent={initialProviderIntent}
            themeMode={themeMode}
            onSetThemeMode={onSetThemeMode}
        />
    );
}));

// Responsive delete-session confirmation. Same split as SettingsShell —
// Drawer anchor=bottom (iOS-style bottom sheet with drag handle) on phones
// and portrait iPad; centred Dialog on desktop and landscape iPad. Replaces
// the browser `window.confirm` which is unstyled, blocks input, and can't
// adapt its layout per viewport.
/** Report a Trash move with Undo, or why it was refused. */


function RenameSessionShell({
    session,
    onClose,
    onConfirm,
}: {
    session: SessionMeta;
    onClose: () => void;
    onConfirm: (title: string) => void;
}): React.JSX.Element {
    const [value, setValue] = useState(session.title);
    const inputRef = useRef<HTMLInputElement>(null);
    const navbarAtBottom = useNavbarAtBottom();
    const desktop = useSurfaceProfile().kind === "desktop";
    // The opener mounts this component with flushSync after synchronously
    // closing the action menu. The hook's layout effect therefore focuses the
    // real input inside the initiating tap, which is the only reliable way to
    // raise iOS's software keyboard for a newly mounted field; on Desktop it
    // claims the field again after the dialog's focus trap has settled.
    useDialogInputFocus(inputRef, desktop);
    // The control sample for the folder name prompt (sheetKeyboardDiagnostics).
    useSheetKeyboardDiagnostics(
        "session-rename",
        !desktop,
        () => inputRef.current?.closest("[role='dialog']") ?? null,
    );
    const trimmed = value.trim();
    const canSave = trimmed.length > 0 && trimmed !== session.title;
    const submit = (): void => {
        if (canSave) onConfirm(trimmed);
    };
    useConfirmEnter(desktop, submit, { suppressBareEnter: false });
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

function SessionInfoShell(
    { session, onClose }: { session: SessionMeta | null; onClose: () => void },
): React.JSX.Element | null {
    const navbarAtBottom = useNavbarAtBottom();
    const [info, setInfo] = useState<SessionInfoPayload | null>(null);
    const [error, setError] = useState(false);
    useEffect(() => {
        if (!session) return undefined;
        setInfo(null);
        setError(false);
        const ctrl = new AbortController();
        void fetch(`/api/sessions/${encodeURIComponent(session.id)}/info`, { signal: ctrl.signal })
            .then((r) => (r.ok ? (r.json() as Promise<SessionInfoPayload>) : Promise.reject(new Error("not found"))))
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
        body = <SessionOverviewRows info={info} live={session} />;
    }
    return (
        <Sheet
            forceSheet={navbarAtBottom}
            cover={navbarAtBottom}
            open
            onClose={onClose}
            title="Session info" islandContext={navbarAtBottom ? { title: session.title || "Session info", status: session.status, facts: [{ label: "Provider", value: session.provider }, { label: "Workspace", value: session.cwd }] } : undefined}
        >
            {body}
        </Sheet>
    );
}
