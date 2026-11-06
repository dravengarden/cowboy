import { AccountTreeOutlined, ArrowBack, ArrowForward, ChatBubbleOutline, CloudOutlined, CheckCircle, CheckCircleOutline, ChevronLeft, ChevronRight, Close, DifferenceOutlined, ExtensionOutlined, FolderOpenOutlined, FolderOutlined, FormatListBulleted, Undo, VisibilityOutlined, WrapText } from "@mui/icons-material";
import { Alert, Badge, Box, Button, Divider, IconButton, ListItemButton, ListItemText, Popover, Stack, Toolbar, Typography } from "@mui/material";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  setActiveSessionId,
  useActiveSessionId,
  useActiveWorkspaceBinding,
  useControlPlaneSessionActivity,
} from "../../controlPlane";
import { importantHaptic, navigationHaptic } from "../../haptic";

import { Sheet } from "../../Sheet";
import { WorkspaceExtensionsButton } from "../../extensions/WorkspaceExtensionsButton";
import { mobileNativeYScrollSx } from "../../mobileNativeOverflow";
import { sessionProjectDirectory } from "../../sessionProject";
import { sessionExecution } from "../../sessionExecution";
import { useAnnouncedSessionContext } from "../../SessionContextCue";
import { inheritedReviewDirectory, resolveReviewDirectory, reviewStateKey, worktreeContextId } from "../../reviewDirectoryDefault";
import { ReviewDirectorySetting } from "../../ReviewDirectorySetting";
import { useRetargetedTouchClickGuard } from "../../useReliableTouchTap";
import {
  mutateMobileReview as mutateReviewState,
  renameSession,
  send,
  trashItems,
  notify,
  openSession,
  useMobileReviewState,
  useStoreSelector,
} from "../../store";
import type { SessionMeta } from "../../protocol";
import { ReviewModeSwitcher } from "./ReviewModeSwitcher";
import { ReviewMoreActions } from "./ReviewMoreActions";
import { ReviewContextButton } from "./ReviewContextButton";
import { SegmentedTabs } from "../../SegmentedTabs";

import { newUuid } from "../../uuid";
import { closeCodeBuffer, CodeApiError, fetchCodeChanges, fetchCodeLanguage, fetchCodeManifest, type GitCommitSummary, isTransientCodeApiStatus, openCodeBuffer } from "./codeApi";
import { invalidateDiffCache } from "./diffCache";
import { reviewEntryKey } from "./diffNavigationModel";
import {
  currentReviewMarks,
  type ReviewProgress,
  revisionMatches,
} from "./reviewProgress";
import { ReviewRepository } from "./ReviewRepository";
import { ReviewCommit } from "./ReviewCommit";
import { bindPullRequest, RemoteReviewApp } from "./RemoteReviewApp";
import { forgetRepositoryPulls } from "./repositoryPulls";
import type { Choice } from "./remoteReviewSources";
import type { PullSummary } from "../../extensions/api";
import type { CodeRevealRange } from "./CodeViewer";
import { ReviewDrawerShell } from "./ReviewDrawerShell";
import { ReviewFileTree } from "./ReviewFileTree";
import { ReviewChanges } from "./ReviewChanges";
import { useReviewOpenRequest } from "../../useReviewOpenRequest";

import { type ReviewBufferMode, reviewBufferMode } from "./ownedReviewBuffer";

import { isMarkdownReviewPath } from "./reviewMarkdown";

import { resolveReviewLink } from "./reviewLinkTarget";

import type { ReviewMode } from "./reviewMode";
import { setReviewLanguageCapabilities, updateReviewSettings, useReviewSettings } from "./reviewSettings";
import { openMobileProduct } from "../appPagerMotion";
import { BOTTOM_NAV_PADDING } from "../bottomNavigation";
import { appendDraftText } from "../../draftStore";

import { type ReviewQuestion, reviewExplainPrompt, reviewExplainSessions, reviewExplainTitle } from "./reviewLineSelection";
import { forkAvailability, forkSession } from "../../sessionFork";
import { requestPickSession } from "../../sessionPickRequest";

import type { CodeLanguage } from "./codeApi";
import type { GitReviewEntry } from "./gitReviewModel";
import { groupGitChanges, reviewQueue } from "./gitReviewModel";
import { ReviewTabStrip } from "./ReviewTabStrip";
import {
  reviewLinkedSessions,
  buildReviewContextProjects,
  orderReviewContextProjects,
  popReviewSessionHistory,
  previousReviewSessionId,
  pushReviewSessionHistory,
  type ReviewContextProject,
  reviewSessionProject,
  registeredReviewWorkspace,
  sessionReviewLocation,
  workspaceCodeContextId,
} from "./reviewContextModel";
import {
  adjacentReviewTabAfterClose,
  closeAllReviewTabs,
  closeOtherReviewTabs,
  closeReviewTab,
  commitFileTabs,
  openReviewTab,
  reorderReviewTabs,
  retainChangedDiffTabs,
  type ReviewTab,
  reviewTabKey,
  toggleReviewTabPin,
} from "./reviewTabs";

import { HintTooltip } from "../../HintTooltip";
import { DocumentView, type ReviewTarget, type SymbolPoint, type SymbolRestoreRequest } from "./DocumentView";
export { DocumentView } from "./DocumentView";

function sessionStatusColor(status: SessionMeta["status"]): string {
  switch (status) {
    case "running":
    case "busy":
      return "success.main";
    case "starting":
      return "warning.main";
    case "crashed":
      return "error.main";
    default:
      return "text.disabled";
  }
}

function ContextSessionRow({
  session,
  selected,
  onPick,
  showWorktree = true,
}: {
  session: SessionMeta;
  selected: boolean;
  onPick: () => void;
  showWorktree?: boolean;
}): React.JSX.Element {
  const execution = sessionExecution(session);
  return (
    <ListItemButton
      selected={selected}
      onClick={onPick}
      sx={{
        minHeight: 64,
        px: 1.25,
        py: 0.75,
        borderRadius: 1.5,
        gap: 1.25,
      }}
    >
      <Box
        aria-label={`${session.status} session`}
        sx={{
          width: 8,
          height: 8,
          flex: "0 0 auto",
          borderRadius: "50%",
          bgcolor: sessionStatusColor(session.status),
        }}
      />
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Stack direction="row" spacing={0.75} alignItems="baseline">
          <Typography
            variant="body2"
            fontWeight={selected ? 700 : 600}
            noWrap
          >
            {session.title}
          </Typography>
          {selected && (
            <Typography variant="caption" color="primary.main">
              Current
            </Typography>
          )}
        </Stack>
        <Typography variant="caption" color="text.secondary" noWrap>
          {execution.machineId ?? (execution.state === "unavailable" ? "Unavailable" : "local")} · {session.provider} ·{" "}
          {reviewSessionProject(session)}
        </Typography>
        {showWorktree && (
          <Typography
            variant="caption"
            color="text.disabled"
            sx={{ display: "block" }}
            noWrap
          >
            {execution.cwd.replace(/\/+$/, "").split("/").at(-1) || execution.cwd}
          </Typography>
        )}
      </Box>
      <ChevronRight color="disabled" fontSize="small" />
    </ListItemButton>
  );
}

type CodeNavigationEntry = Extract<ReviewTarget, { kind: "source" }> & {
  symbol?: SymbolPoint;
  navigationRange?: Omit<CodeRevealRange, "id">;
};

type TabCloseRequest =
  | { kind: "one"; key: string; anchor: HTMLElement }
  | { kind: "others"; key: string; anchor: HTMLElement }
  | { kind: "all"; anchor: HTMLElement };

export function ReviewApp({
  active: surfaceActive,
  onDrawerOpenChange,
}: {
  active: boolean;
  onDrawerOpenChange: (open: boolean) => void;
}): React.JSX.Element {
  const boundWorkspace = useActiveWorkspaceBinding();
  const activeSessionId = useActiveSessionId();
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const currentSession = sessions.find((session) => session.id === boundWorkspace?.sessionId);
  const execution = currentSession ? sessionExecution(currentSession) : undefined;
  const executionUsable = execution?.state === "ready" || execution?.state === "local";
  const machineInventories = useStoreSelector((snapshot) => snapshot.machines);
  const sessionMachine = machineInventories.find((machine) =>
    execution?.machineId ? machine.id === execution.machineId
      : execution?.state === "unavailable" ? false : machine.local
  );
  const sessionRegisteredWorkspace = currentSession
    ? registeredReviewWorkspace(currentSession, (sessionMachine?.workspaces ?? []).map((workspace) => ({
      id: workspace.id, displayName: workspace.display_name, canonicalPath: workspace.canonical_path,
    })))
    : undefined;
  const sessionLocation = currentSession ? sessionReviewLocation(currentSession, sessionRegisteredWorkspace) : undefined;
  const folderTree = useStoreSelector(state => state.sessionFolders);
  const sessionReview = useMobileReviewState(currentSession?.id);
  const defaultsReady = useStoreSelector(state => state.sessionFoldersReady && (!currentSession || state.mobileReviewReady.has(currentSession.id)));
  const mutateMobileReview: typeof mutateReviewState = useCallback((context, name, args) => mutateReviewState(reviewStateKey(boundWorkspace?.sessionId, context), name, args), [boundWorkspace?.sessionId]);
  const directoryDefault = currentSession ? inheritedReviewDirectory(currentSession, folderTree, sessionReview.directory) : undefined;
  const [readingDefault, setReadingDefault] = useState<{ owner: string; value: string | null | undefined }>();
  const directoryValue = readingDefault?.owner === currentSession?.id && readingDefault
    ? readingDefault.value : directoryDefault?.value;
  const resolvedDirectory = !defaultsReady ? { error: "Loading directory defaults…" } : currentSession ? resolveReviewDirectory(directoryValue, currentSession, sessions, machineInventories) : {};
  const boundCodeWorkspace = boundWorkspace
    ? { ...boundWorkspace, cwd: executionUsable ? execution?.cwd ?? "" : "" }
    : null;
  const [selectedProjectContext, setProjectCodeContext] = useState<
    {
      ownerSessionId: string | null;
      sessionId: string;
      cwd: string;
      provider: string;
      title: string;
      machineId: string;
      workspaceId?: string;
      project: string;
      kind?: "Worktree" | "Project code" | "Workspace";
    } | undefined
  >();
  useReviewOpenRequest(currentSession?.id, () => {
    if (!currentSession) return;
    setProjectCodeContext(undefined);
    setReadingDefault({ owner: currentSession.id, value: `session::${currentSession.id}` });
  });
  // A context override belongs to the session in which it was selected. Never
  // issue even one read for that project under a newly selected session.
  const explicitProjectContext = selectedProjectContext?.ownerSessionId === (boundWorkspace?.sessionId ?? null)
    ? selectedProjectContext
    : undefined;
  const projectCodeContext = explicitProjectContext ??
    (resolvedDirectory.context && resolvedDirectory.context.sessionId !== currentSession?.id
      ? { ...resolvedDirectory.context, ownerSessionId: currentSession?.id ?? null } : undefined);
  const workspace = projectCodeContext ??
    (!resolvedDirectory.error && boundCodeWorkspace && (boundCodeWorkspace.cwd || !executionUsable) ? boundCodeWorkspace : null);
  const workspaceIdentity = workspace
    ? JSON.stringify([workspace.sessionId, workspace.cwd, projectCodeContext?.machineId ?? execution?.machineId])
    : undefined;
  const syncedReview = useMobileReviewState(workspace ? reviewStateKey(boundWorkspace?.sessionId, workspace.sessionId) : undefined);
  // Pin the preference while a file is open. Parent edits update the default
  // shown in settings without reinterpreting a relative tab in a new root.
  useEffect(() => {
    if (defaultsReady && surfaceActive && currentSession && workspace && !explicitProjectContext && (syncedReview.active || syncedReview.remote_selected)) {
      setReadingDefault(previous => previous?.owner === currentSession.id ? previous : { owner: currentSession.id, value: directoryValue });
    }
  }, [defaultsReady, surfaceActive, currentSession?.id, workspaceIdentity, syncedReview.active, syncedReview.remote_selected, directoryValue, explicitProjectContext]);
  const remoteSelected = !projectCodeContext && syncedReview.remote_selected === true;
  const active = surfaceActive && !remoteSelected;
  useEffect(() => {
    if (remoteSelected) onDrawerOpenChange(false);
  }, [remoteSelected, onDrawerOpenChange]);
  const controlPlaneActivity = useControlPlaneSessionActivity(
    workspace?.sessionId,
  );
  const settings = useReviewSettings();
  const [mode, setMode] = useState<ReviewMode>("git");
  const [markdownPreview, setMarkdownPreview] = useState(true);
  const [sourceState, setSourceTarget] = useState<
    Extract<ReviewTarget, { kind: "source" }> | undefined
  >();
  const [diffState, setDiffTarget] = useState<
    Extract<ReviewTarget, { kind: "diff" }> | undefined
  >();
  const [commitState, setCommitTarget] = useState<{
    commit: GitCommitSummary;
    path?: string;
  }>();
  const documentContext = useRef(workspaceIdentity);
  const sameDocumentContext = documentContext.current === workspaceIdentity;
  const sourceTarget = sameDocumentContext ? sourceState : undefined;
  const diffTarget = sameDocumentContext ? diffState : undefined;
  const commitTarget = sameDocumentContext ? commitState : undefined;
  const [commitPaths, setCommitPaths] = useState<readonly string[]>([]);
  const target: ReviewTarget = mode === "files"
    ? sourceTarget ?? { kind: "changes" }
    : diffTarget ?? { kind: "changes" };
  const leasedPath = remoteSelected || target.kind === "changes" ? undefined : target.path;
  const [closeRequest, setCloseRequest] = useState(0);
  const [toggleDrawerRequest, setToggleDrawerRequest] = useState(0);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [sessionSwitcherOpen, setSessionSwitcherOpen] = useState(false);
  const [directoryEditing, setDirectoryEditing] = useState(false);
  const [contextTab, setContextTab] = useState<"sessions" | "projects">(
    "sessions",
  );
  const [contextProjectKey, setContextProjectKey] = useState<string>();
  const [inlineNavigationContainer, setInlineNavigationContainer] = useState<HTMLElement | null>(null);
  const [inlineActionsContainer, setInlineActionsContainer] = useState<HTMLElement | null>(null);
  const navigationInline = !!workspace && target.kind === "changes" && !commitTarget;
  const navigationTouchGuard = useRetargetedTouchClickGuard<HTMLDivElement>();
  const [contextError, setContextError] = useState<string>();
  const [sessionHistory, setSessionHistory] = useState<string[]>([]);
  const sessionListRef = useRef<HTMLDivElement>(null);
  const projectContextSessionRef = useRef(activeSessionId);
  const observedSessionRef = useRef<string | undefined>(undefined);
  const pullBindRef = useRef<AbortController | undefined>(undefined);
  const suppressedHistoryTargetRef = useRef<string | undefined>(undefined);
  const [reviewProgress, setReviewProgress] = useState<ReviewProgress>({});
  const [currentRevision, setCurrentRevision] = useState<string>();
  const [dataRevision, setDataRevision] = useState(0);
  const [changeCount, setChangeCount] = useState(0);
  const [repositoryContext, setRepositoryContext] = useState<{
    project: string;
    branch: string | undefined;
    worktree: string | undefined;
    head: string | undefined;
  }>();
  const [languageData, setLanguageData] = useState<CodeLanguage>();
  const [bufferSelection, setBufferSelection] = useState<
    { sessionId: string; mode: ReviewBufferMode }
  >();
  const bufferMode = bufferSelection?.sessionId === workspace?.sessionId
    ? bufferSelection?.mode
    : undefined;
  const [tabs, setTabs] = useState<ReviewTab[]>([]);
  const tabScrollPositions = useRef(new Map<string, unknown>());
  const readScrollPosition = useCallback(
    (key: string): unknown => tabScrollPositions.current.get(key),
    [],
  );
  const rememberScrollPosition = useCallback(
    (key: string, top: number): void => {
      if (!Number.isFinite(top) || top < 0) return;
      tabScrollPositions.current.set(key, top);
    },
    [],
  );
  const [tabCloseRequest, setTabCloseRequest] = useState<TabCloseRequest>();
  const [gitQueue, setGitQueue] = useState<GitReviewEntry[]>([]);
  const [gitComparison, setGitComparison] = useState<
    string | null | undefined
  >(undefined);
  const [previewAnchor, setPreviewAnchor] = useState<
    { path: string; hash: string; id: number } | undefined
  >(undefined);
  const previewAnchorId = useRef(0);
  const [navigationHistory, setNavigationHistory] = useState<
    CodeNavigationEntry[]
  >([]);
  const [navigationForwardHistory, setNavigationForwardHistory] = useState<
    CodeNavigationEntry[]
  >([]);
  const [symbolRestore, setSymbolRestore] = useState<SymbolRestoreRequest>();
  const [closeSymbolRequest, setCloseSymbolRequest] = useState(0);
  const currentSymbol = useRef<SymbolPoint | undefined>(undefined);
  const currentVisibleLine = useRef<number | undefined>(undefined);
  const currentNavigationFrame = useRef<CodeNavigationEntry | undefined>(
    undefined,
  );
  const symbolRestoreId = useRef(0);
  const revealRangeId = useRef(0);
  const [outlineOpen, setOutlineOpen] = useState(false);
  const syncedReviewRef = useRef(syncedReview);
  syncedReviewRef.current = syncedReview;
  const [manifestRefreshRequest, setManifestRefreshRequest] = useState(0);
  const [bufferRecoveryRequest, setBufferRecoveryRequest] = useState(0);
  const requestBufferRecovery = useCallback((): void => {
    setBufferRecoveryRequest((request) => request + 1);
  }, []);
  const manifestRevision = useRef<string | undefined>(undefined);
  const adoptManifestRevision = useCallback((revision: string): void => {
    manifestRevision.current = revision;
  }, []);
  const handleDrawerOpenChange = useCallback((open: boolean): void => {
    setDrawerOpen(open);
    onDrawerOpenChange(open);
  }, [onDrawerOpenChange]);
  const adoptGitComparison = useCallback(
    (comparison: string | null): void => {
      setGitComparison(comparison);
      setGitQueue([]);
      setDiffTarget(undefined);
      setCurrentRevision(undefined);
      if (workspace?.sessionId) invalidateDiffCache(workspace.sessionId);
    },
    [workspace?.sessionId],
  );
  const comparisonStateId = workspace
    ? reviewStateKey(boundWorkspace?.sessionId, workspace.sessionId)
    : undefined;
  const changeGitComparison = useCallback(
    (comparison: string | null): void => {
      adoptGitComparison(comparison);
      if (comparisonStateId) {
        mutateReviewState(comparisonStateId, "setComparison", {
          comparison: comparison ?? "",
        });
      }
    },
    [adoptGitComparison, comparisonStateId],
  );
  const sendSelectionToAgent = useMemo(() => {
    if (!activeSessionId) return undefined;
    return (message: string): void => {
      appendDraftText(activeSessionId, message);
      openMobileProduct("agent");
    };
  }, [activeSessionId]);
  const explainSource = currentSession &&
      forkAvailability(currentSession, machineInventories) !== null
    ? currentSession.id
    : undefined;
  const explainRoot = projectCodeContext?.cwd;
  const explainSelection = useMemo(() => {
    if (!explainSource) return undefined;
    return async (path: string, message: string, kind: ReviewQuestion): Promise<void> => {
      // One Explain per session: follow-ups stay in it until the Agent moves
      // on, and then a new fork takes over and the old one goes to Trash.
      const earlier = reviewExplainSessions(sessions, explainSource);
      const fork = await forkSession(explainSource, { latestCompleted: true, reuse: earlier.at(-1)?.id });
      const reused = earlier.some((session) => session.id === fork.session_id);
      if (!reused) {
        renameSession(fork.session_id, reviewExplainTitle(path));
        // An Explain still answering stays, and the next one clears it.
        if (earlier.length > 0) {
          void trashItems({ sessions: earlier.map((session) => session.id) }).catch(() => undefined);
        }
      }
      const prompt = reviewExplainPrompt(message, explainRoot, kind);
      if (!send({ type: "submit", session_id: fork.session_id, text: prompt, cmid: `explain-${crypto.randomUUID()}` })) {
        // Nothing is lost offline: the question waits in the fork's Composer.
        appendDraftText(fork.session_id, prompt);
      }
      requestPickSession(fork.session_id, fork.session ?? undefined);
      openMobileProduct("agent");
    };
  }, [explainRoot, explainSource, sessions]);
  const preferredGitComparison = syncedReview.comparison === undefined
    ? undefined
    : syncedReview.comparison || null;

  useEffect(() => {
    if (
      !workspace?.sessionId ||
      !leasedPath || !bufferMode || bufferMode === "unavailable" ||
      bufferMode === "owned"
    ) {
      setLanguageData(undefined);
      return undefined;
    }
    const sessionId = workspace.sessionId;
    const path = leasedPath;
    const leaseId = newUuid();
    let released = false;
    let opened = false;
    let openAttempt = 0;
    const retryTimers: number[] = [];
    const release = (): void => {
      if (!opened) return;
      opened = false;
      void closeCodeBuffer(sessionId, path, leaseId).catch(() => undefined);
    };
    const loadLanguage = (): void => {
      void fetchCodeLanguage(sessionId, path)
        .then((value) => {
          if (!released) setLanguageData(value);
        })
        // File rendering remains useful while a language server is cold or
        // unavailable.
        .catch(() => undefined);
    };
    const acquire = (): void => {
      const attempt = openAttempt++;
      void openCodeBuffer(sessionId, path, leaseId)
        .then(() => {
          opened = true;
          if (released) {
            release();
            return;
          }
          loadLanguage();
          // Zed starts an LSP only after the first buffer registration. Keep
          // the source visible immediately, then revalidate after typical warm
          // and cold language-server startup windows.
          retryTimers.push(globalThis.setTimeout(loadLanguage, 8_000));
          retryTimers.push(globalThis.setTimeout(loadLanguage, 30_000));
        })
        .catch((reason) => {
          const status = reason instanceof CodeApiError
            ? reason.status
            : undefined;
          if (
            released || attempt >= 3 ||
            !isTransientCodeApiStatus(status)
          ) return;
          const delay = [1_000, 4_000, 12_000][attempt] ?? 12_000;
          retryTimers.push(globalThis.setTimeout(acquire, delay));
        });
    };
    acquire();
    return () => {
      released = true;
      retryTimers.forEach((timer) => globalThis.clearTimeout(timer));
      setLanguageData(undefined);
      release();
    };
  }, [
    bufferRecoveryRequest,
    dataRevision,
    leasedPath,
    workspace?.sessionId,
    bufferMode,
    target.kind,
  ]);

  useEffect(() => {
    setTabCloseRequest(undefined);
    manifestRevision.current = undefined;
    setDataRevision(0);
    setChangeCount(0);
    setRepositoryContext(undefined);
    setGitQueue([]);
    setGitComparison(undefined);
    setReviewLanguageCapabilities(undefined);
    setBufferSelection(undefined);
  }, [workspaceIdentity]);

  useEffect(() => {
    if (!active || !workspace?.sessionId) return undefined;
    // ACP streams can emit many updates for one tool call. Coalesce them into
    // one filesystem revalidation after the burst instead of repeatedly
    // running git status while text is still streaming.
    const timer = globalThis.setTimeout(
      () => setManifestRefreshRequest((value) => value + 1),
      600,
    );
    return () => globalThis.clearTimeout(timer);
  }, [active, controlPlaneActivity, workspace?.sessionId]);

  useEffect(() => {
    if (!active || !workspace?.sessionId) return undefined;
    let controller: AbortController | undefined;
    const refreshManifest = (): void => {
      controller?.abort();
      controller = new AbortController();
      const observer = controller;
      void fetchCodeManifest(workspace.sessionId, observer.signal)
        .then((manifest) => {
          if (observer.signal.aborted) return undefined;
          setBufferSelection((current) => ({
            sessionId: workspace.sessionId,
            mode: reviewBufferMode(
              current?.sessionId === workspace.sessionId
                ? current.mode
                : undefined,
              manifest.bufferMode,
            ),
          }));
          setChangeCount(manifest.changeCount);
          setRepositoryContext({
            project: manifest.project,
            branch: manifest.branch,
            worktree: manifest.worktree,
            head: manifest.head,
          });
          setReviewLanguageCapabilities(manifest.language);
          const previous = manifestRevision.current;
          manifestRevision.current = manifest.revision;
          if (previous && previous !== manifest.revision) {
            invalidateDiffCache(workspace.sessionId);
            setDataRevision((value) => value + 1);
          }
          return fetchCodeChanges(
            workspace.sessionId,
            observer.signal,
            gitComparison ?? undefined,
          );
        })
        .then((changes) => {
          if (observer.signal.aborted || !changes) return;
          setGitQueue(
            reviewQueue(
              groupGitChanges(changes.changes, gitComparison ?? undefined),
            ),
          );
        })
        // The ordinary tree/changes error surfaces remain authoritative.
        .catch(() => undefined);
    };
    refreshManifest();
    // Zed/control-plane activity handles the ordinary fast path. Retain a low
    // frequency check for files changed by shells, editors, or Git outside
    // Cowboy, and revalidate immediately when Mobile returns to the foreground.
    const timer = globalThis.setInterval(refreshManifest, 30_000);
    const refreshVisible = (): void => {
      if (document.visibilityState === "visible") refreshManifest();
    };
    document.addEventListener("visibilitychange", refreshVisible);
    globalThis.addEventListener("online", refreshManifest);
    return () => {
      globalThis.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshVisible);
      globalThis.removeEventListener("online", refreshManifest);
      controller?.abort();
    };
  }, [active, gitComparison, manifestRefreshRequest, workspace?.sessionId, workspaceIdentity]);

  useEffect(() => {
    documentContext.current = workspaceIdentity;
    setGitComparison(undefined);
    setGitQueue([]);
    setSourceTarget(undefined);
    setNavigationHistory([]);
    setNavigationForwardHistory([]);
    setSymbolRestore(undefined);
    currentSymbol.current = undefined;
    currentVisibleLine.current = undefined;
    setDiffTarget(undefined);
    setCommitTarget(undefined);
    setCurrentRevision(undefined);
  }, [workspaceIdentity]);

  useEffect(() => {
    setMode(syncedReview.mode);
    if (syncedReview.mode === "files") setCommitTarget(undefined);
  }, [syncedReview.mode, workspace?.sessionId]);

  useEffect(() => {
    setTabs(syncedReview.tabs.map((tab) => ({ kind: "source", ...tab })));
  }, [syncedReview.tabs, workspace?.sessionId]);

  useEffect(() => {
    setReviewProgress({ ...syncedReview.progress });
  }, [syncedReview.progress, workspace?.sessionId]);

  useEffect(() => {
    const path = syncedReview.active;
    if (!path) {
      setSourceTarget(undefined);
      return;
    }
    setMarkdownPreview(isMarkdownReviewPath(path));
    const line = syncedReviewRef.current.positions?.[path]?.line;
    setCloseRequest((value) => value + 1);
    setSourceTarget((current) =>
      current?.path === path ? current : {
        kind: "source",
        path,
        ...(line === undefined ? {} : {
          revealLine: line,
          revealRequestId: ++revealRangeId.current,
        }),
      }
    );
  }, [syncedReview.active, syncedReview.open_revision, workspace?.sessionId]);

  const openSource = (
    path: string,
    revealLine?: number,
    preserveNavigation = false,
    revealRange?: Omit<CodeRevealRange, "id">,
  ): void => {
    if (currentSession && !explicitProjectContext) setReadingDefault({ owner: currentSession.id, value: directoryValue });
    const revealRequestId = revealLine !== undefined || revealRange
      ? ++revealRangeId.current
      : undefined;
    currentVisibleLine.current = revealLine;
    if (!preserveNavigation) {
      currentNavigationFrame.current = {
        kind: "source",
        path,
        ...(revealLine === undefined ? {} : { revealLine }),
      };
    }
    setCurrentRevision(undefined);
    setCommitTarget(undefined);
    setMode("files");
    if (workspace?.sessionId) {
      mutateMobileReview(workspace.sessionId, "open", { path });
    }
    setMarkdownPreview(isMarkdownReviewPath(path));
    setSourceTarget({
      kind: "source",
      path,
      ...(revealLine === undefined ? {} : { revealLine }),
      ...(revealRequestId === undefined ? {} : { revealRequestId }),
      ...(revealRange
        ? { revealRange: { ...revealRange, id: revealRequestId! } }
        : {}),
    });
    setTabs((current) =>
      openReviewTab(current, { kind: "source", path, pinned: false })
    );
    if (!preserveNavigation) {
      setNavigationHistory([]);
      setNavigationForwardHistory([]);
      setSymbolRestore(undefined);
      currentSymbol.current = undefined;
    }
    setCloseRequest((value) => value + 1);
  };
  const restoreNavigationEntry = (entry: CodeNavigationEntry): void => {
    currentNavigationFrame.current = entry;
    currentSymbol.current = undefined;
    currentVisibleLine.current = entry.revealLine;
    setCloseSymbolRequest((value) => value + 1);
    if (entry.symbol) {
      setSymbolRestore({
        path: entry.path,
        ...entry.symbol,
        id: ++symbolRestoreId.current,
      });
    } else {
      setSymbolRestore(undefined);
    }
    const row = Math.max(0, (entry.revealLine ?? 1) - 1);
    openSource(
      entry.path,
      entry.revealLine,
      true,
      entry.navigationRange ?? {
        start: entry.symbol ?? { row, column: 0 },
        end: entry.symbol
          ? { row: entry.symbol.row, column: entry.symbol.column + 1 }
          : { row, column: Number.MAX_SAFE_INTEGER },
      },
    );
  };
  const currentNavigationEntry = (): CodeNavigationEntry | undefined => {
    if (target.kind !== "source") return undefined;
    const frame = currentNavigationFrame.current;
    if (frame?.path === target.path) return frame;
    return {
      kind: "source",
      path: target.path,
      ...(currentVisibleLine.current === undefined
        ? {}
        : { revealLine: currentVisibleLine.current }),
    };
  };
  /**
   * Follow a link inside rendered Markdown.
   *
   * Deliberately reuses the go-to-definition stack rather than inventing a
   * second history: a doc link and a symbol jump are the same act — "I left
   * where I was" — and a reader must not have to remember which Back a given
   * jump answers to. The Back/Forward pair in the header therefore lights up
   * for documentation exactly as it does for code, and the per-path scroll
   * memory returns the previous document to the paragraph it was left at.
   *
   * Returns true when this navigated, so the renderer suppresses the anchor.
   */
  const followMarkdownLinkNow = (href: string): boolean => {
    if (target.kind !== "source") return false;
    const resolved = resolveReviewLink(target.path, href);
    if (resolved.kind === "external") return false;
    if (resolved.kind === "unsupported") {
      // Claimed and reported: falling through would resolve it against the
      // app's own origin and open a page that cannot exist.
      notify("That link points outside this workspace.", "warning");
      return true;
    }
    if (resolved.kind === "anchor") {
      navigationHaptic();
      setPreviewAnchor({
        path: target.path,
        hash: resolved.hash,
        id: ++previewAnchorId.current,
      });
      return true;
    }
    if (resolved.path === target.path) {
      // Same document: a heading move, not a navigation entry.
      if (resolved.hash) {
        setPreviewAnchor({
          path: target.path,
          hash: resolved.hash,
          id: ++previewAnchorId.current,
        });
      }
      return true;
    }
    navigationHaptic();
    const previous = currentNavigationEntry();
    if (previous) {
      setNavigationHistory((history) => [...history, previous].slice(-32));
    }
    setNavigationForwardHistory([]);
    setSymbolRestore(undefined);
    setCloseSymbolRequest((value) => value + 1);
    currentSymbol.current = undefined;
    currentNavigationFrame.current = {
      kind: "source",
      path: resolved.path,
      ...(resolved.line === undefined ? {} : { revealLine: resolved.line }),
    };
    setPreviewAnchor(
      resolved.hash === undefined ? undefined : {
        path: resolved.path,
        hash: resolved.hash,
        id: ++previewAnchorId.current,
      },
    );
    openSource(resolved.path, resolved.line, true);
    return true;
  };
  // The rendered preview must keep one identity across Review renders. A new
  // callback busts the Markdown memo and remounts every paragraph and link;
  // iOS then keeps delivering an in-flight drawer swipe to the detached node,
  // the drawer never sees touchend, and the peek sticks part-way open.
  const followMarkdownLinkRef = useRef(followMarkdownLinkNow);
  followMarkdownLinkRef.current = followMarkdownLinkNow;
  const followMarkdownLink = useCallback(
    (href: string): boolean => followMarkdownLinkRef.current(href),
    [],
  );
  const navigateBack = (): void => {
    const previous = navigationHistory.at(-1);
    if (!previous) return;
    navigationHaptic();
    const current = currentNavigationEntry();
    if (current) {
      setNavigationForwardHistory((history) =>
        [...history, current].slice(-32)
      );
    }
    setNavigationHistory((history) => history.slice(0, -1));
    restoreNavigationEntry(previous);
  };
  const navigateForward = (): void => {
    const next = navigationForwardHistory.at(-1);
    if (!next) return;
    navigationHaptic();
    const current = currentNavigationEntry();
    if (current) {
      setNavigationHistory((history) => [...history, current].slice(-32));
    }
    setNavigationForwardHistory((history) => history.slice(0, -1));
    restoreNavigationEntry(next);
  };
  const openDiff = (
    entry: GitReviewEntry,
    queue: GitReviewEntry[],
  ): void => {
    if (currentSession && !explicitProjectContext) setReadingDefault({ owner: currentSession.id, value: directoryValue });
    setCurrentRevision(undefined);
    setCommitTarget(undefined);
    setMode("git");
    if (workspace?.sessionId) {
      mutateMobileReview(workspace.sessionId, "setMode", { mode: "git" });
    }
    setDiffTarget({
      kind: "diff",
      path: entry.change.path,
      scope: entry.scope,
      ...(entry.comparison ? { comparison: entry.comparison } : {}),
      queue,
    });
    setCloseRequest((value) => value + 1);
  };
  const openCommit = (commit: GitCommitSummary): void => {
    if (currentSession && !explicitProjectContext) setReadingDefault({ owner: currentSession.id, value: directoryValue });
    navigationHaptic();
    setMode("git");
    setDiffTarget(undefined);
    setCurrentRevision(undefined);
    setCommitTarget({ commit });
    if (workspace?.sessionId) {
      mutateMobileReview(workspace.sessionId, "setMode", { mode: "git" });
    }
  };
  const activateTab = (tab: ReviewTab): void => {
    if (tab.kind === "source") {
      openSource(tab.path);
      return;
    }
    if (tab.kind === "commit") {
      setCommitTarget((current) =>
        current ? { commit: current.commit, path: tab.path } : current
      );
      return;
    }
    const entry = gitQueue.find((candidate) =>
      candidate.change.path === tab.path && candidate.scope === tab.scope &&
      candidate.comparison === tab.comparison
    );
    if (entry) openDiff(entry, gitQueue);
  };
  const activeTabKey = commitTarget?.path
    ? reviewTabKey({ kind: "commit", path: commitTarget.path, pinned: false })
    : target.kind === "source"
    ? reviewTabKey({ ...target, pinned: false })
    : target.kind === "diff"
    ? reviewTabKey({ ...target, pinned: false })
    : undefined;
  const gitTabs: ReviewTab[] = gitQueue.map((entry): ReviewTab => {
    const key = reviewTabKey({
      kind: "diff",
      path: entry.change.path,
      scope: entry.scope,
      ...(entry.comparison ? { comparison: entry.comparison } : {}),
      pinned: false,
    });
    return {
      kind: "diff",
      path: entry.change.path,
      scope: entry.scope,
      ...(entry.comparison ? { comparison: entry.comparison } : {}),
      pinned: tabs.find((tab) => reviewTabKey(tab) === key)?.pinned ?? false,
    };
  }).sort((left, right) => Number(right.pinned) - Number(left.pinned));
  const modeTabs = mode === "files"
    ? tabs.filter((tab) => tab.kind === "source")
    : gitTabs;
  const activateOrCollapseTab = (tab: ReviewTab): void => {
    if (reviewTabKey(tab) === activeTabKey) {
      navigationHaptic();
      if (tab.kind === "source") setSourceTarget(undefined);
      else if (tab.kind === "commit") {
        setCommitTarget((current) =>
          current ? { commit: current.commit } : current
        );
      } else setDiffTarget(undefined);
      return;
    }
    activateTab(tab);
  };
  useEffect(() => {
    const changedKeys = new Set(
      gitQueue.map((entry) =>
        reviewTabKey({
          kind: "diff",
          path: entry.change.path,
          scope: entry.scope,
          ...(entry.comparison ? { comparison: entry.comparison } : {}),
          pinned: false,
        })
      ),
    );
    setTabs((current) => retainChangedDiffTabs(current, changedKeys));
  }, [gitQueue]);
  const closeTab = (key: string): void => {
    const closing = tabs.find((tab) => reviewTabKey(tab) === key);
    const fallback = adjacentReviewTabAfterClose(tabs, key);
    if (closing?.kind === "source" && workspace?.sessionId) {
      mutateMobileReview(workspace.sessionId, "close", { path: closing.path });
    }
    const next = closeReviewTab(tabs, key);
    setTabs(next);
    if (activeTabKey !== key) return;
    if (fallback) activateTab(fallback);
    else if (mode === "files") setSourceTarget(undefined);
    else setDiffTarget(undefined);
  };
  const closeOtherTabs = (key: string): void => {
    if (workspace?.sessionId) {
      for (const tab of modeTabs) {
        if (
          tab.kind === "source" &&
          reviewTabKey(tab) !== key &&
          !tab.pinned
        ) {
          mutateMobileReview(workspace.sessionId, "close", { path: tab.path });
        }
      }
    }
    const keep = modeTabs.find((tab) => reviewTabKey(tab) === key);
    setTabs((current) => {
      const currentModeTabs = current.filter((tab) => tab.kind === "source");
      const otherModeTabs = current.filter((tab) => tab.kind !== "source");
      return [
        ...otherModeTabs,
        ...closeOtherReviewTabs(currentModeTabs, key),
      ];
    });
    if (keep) activateTab(keep);
  };
  const closeAllTabs = (): void => {
    if (workspace?.sessionId) {
      for (const tab of modeTabs) {
        if (tab.kind === "source") {
          mutateMobileReview(workspace.sessionId, "close", { path: tab.path });
        }
      }
    }
    setTabs((current) => closeAllReviewTabs(current, "source"));
    setSourceTarget(undefined);
    setNavigationHistory([]);
    setNavigationForwardHistory([]);
    setSymbolRestore(undefined);
    currentSymbol.current = undefined;
  };
  const requestCloseTab = (
    key: string,
    anchor: HTMLElement,
  ): void => {
    navigationHaptic();
    setTabCloseRequest({ kind: "one", key, anchor });
  };
  const confirmTabCloseRequest = (): void => {
    const request = tabCloseRequest;
    if (!request) return;
    setTabCloseRequest(undefined);
    importantHaptic();
    if (request.kind === "one") closeTab(request.key);
    else if (request.kind === "others") closeOtherTabs(request.key);
    else closeAllTabs();
  };
  const requestedTab = tabCloseRequest?.kind === "one"
    ? tabs.find((tab) => reviewTabKey(tab) === tabCloseRequest.key)
    : undefined;
  const tabCloseCount = tabCloseRequest?.kind === "others"
    ? modeTabs.filter((tab) =>
      reviewTabKey(tab) !== tabCloseRequest.key && !tab.pinned
    ).length
    : tabCloseRequest?.kind === "all"
    ? modeTabs.length
    : 1;
  const tabCloseTitle = tabCloseRequest?.kind === "one"
    ? `Close ${requestedTab?.path.split("/").at(-1) ?? "this tab"}?`
    : tabCloseRequest?.kind === "others"
    ? `Close ${tabCloseCount} other ${tabCloseCount === 1 ? "tab" : "tabs"}?`
    : tabCloseCount === 1
    ? "Close the only open tab?"
    : `Close all ${tabCloseCount} tabs?`;
  const tabCloseDetail = tabCloseRequest?.kind === "others"
    ? "Other unpinned tabs will close. Pinned tabs stay open."
    : tabCloseRequest?.kind === "all"
    ? "Every open file tab will close."
    : "This tab will close.";
  const tabCloseAction = tabCloseRequest?.kind === "others"
    ? "Close others"
    : tabCloseRequest?.kind === "all"
    ? "Close all"
    : "Close tab";
  const reviewIndex = target.kind === "diff"
    ? target.queue.findIndex((entry) =>
      entry.change.path === target.path && entry.scope === target.scope
        && entry.comparison === target.comparison
    )
    : -1;
  useEffect(() => {
    if (target.kind !== "diff") return;
    const stillChanged = gitQueue.some((entry) =>
      entry.change.path === target.path && entry.scope === target.scope
        && entry.comparison === target.comparison
    );
    if (!stillChanged) {
      setDiffTarget(undefined);
      setCurrentRevision(undefined);
    }
  }, [gitQueue, target]);
  const moveReview = (offset: number): void => {
    if (target.kind !== "diff") return;
    const entry = target.queue[reviewIndex + offset];
    if (entry) openDiff(entry, target.queue);
  };
  const targetReviewKey = target.kind === "diff"
    ? reviewEntryKey(target.path, target.scope, target.comparison)
    : undefined;
  // Prefer the live list's fingerprint: it tracks the file, not one diff page.
  const targetFingerprint = target.kind === "diff"
    ? gitQueue.find((entry) =>
      reviewEntryKey(entry.change.path, entry.scope, entry.comparison) ===
        targetReviewKey
    )?.change.fingerprint
    : undefined;
  const reviewMark = targetFingerprint ?? currentRevision;
  const targetIsReviewed = targetReviewKey
    ? revisionMatches(reviewProgress, targetReviewKey, reviewMark)
    : false;
  const reviewedMarks = useMemo(
    () =>
      currentReviewMarks(
        reviewProgress,
        gitQueue.map((entry) => ({
          key: reviewEntryKey(entry.change.path, entry.scope, entry.comparison),
          fingerprint: entry.change.fingerprint,
        })),
      ),
    [gitQueue, reviewProgress],
  );
  useEffect(() => {
    if (
      !workspace?.sessionId ||
      !targetReviewKey ||
      !reviewMark ||
      reviewProgress[targetReviewKey] === undefined ||
      targetIsReviewed
    ) {
      return;
    }
    const next = { ...reviewProgress };
    delete next[targetReviewKey];
    setReviewProgress(next);
    mutateMobileReview(workspace.sessionId, "markReviewed", {
      key: targetReviewKey,
      revision: null,
    });
  }, [
    reviewMark,
    reviewProgress,
    targetIsReviewed,
    targetReviewKey,
    workspace?.sessionId,
  ]);
  const toggleReviewed = (): void => {
    if (!targetReviewKey || !reviewMark || !workspace?.sessionId) return;
    const next = { ...reviewProgress };
    if (targetIsReviewed) delete next[targetReviewKey];
    else next[targetReviewKey] = reviewMark;
    setReviewProgress(next);
    mutateMobileReview(workspace.sessionId, "markReviewed", {
      key: targetReviewKey,
      revision: targetIsReviewed ? null : reviewMark,
    });
  };
  const currentProject = projectCodeContext?.project ??
    sessionLocation?.project ?? repositoryContext?.project;
  const contextLabel = projectCodeContext ? projectCodeContext.kind ?? "Project code" : sessionLocation?.kind ?? "Worktree";
  const currentMachineId = projectCodeContext?.machineId ??
    execution?.machineId ?? sessionMachine?.id ?? (execution?.state === "unavailable" ? "unavailable" : "local");
  const displayedChangeCount = gitComparison
    ? gitQueue.length
    : changeCount;
  const currentMachineInventory = machineInventories.find((machine) =>
    machine.id === currentMachineId
  );
  const currentRegisteredWorkspace = projectCodeContext
    ? currentMachineInventory?.workspaces.find((registered) => registered.id === projectCodeContext.workspaceId)
    : currentMachineInventory?.workspaces.find((registered) => registered.id === sessionRegisteredWorkspace?.id);
  const localMachineId = machineInventories.find((machine) => machine.local)?.id;
  const currentProjectPath = projectCodeContext?.cwd ?? sessionProjectDirectory(
    currentSession,
    currentRegisteredWorkspace?.canonical_path,
  ) ?? undefined;
  const reviewRootPath = workspace?.cwd || (projectCodeContext ? currentProjectPath : undefined);
  const contextProjects = useMemo(
    () =>
      orderReviewContextProjects(
        buildReviewContextProjects(
          sessions,
          currentMachineId,
          (currentMachineInventory?.workspaces ?? []).map((registered) => ({
            id: registered.id,
            displayName: registered.display_name,
            canonicalPath: registered.canonical_path,
          })),
          localMachineId,
        ),
        currentProject,
        currentMachineId,
      ),
    [
      currentMachineId,
      currentMachineInventory?.workspaces,
      currentProject,
      sessions,
      localMachineId,
    ],
  );
  const selectedContextProject = contextProjects.find((project) =>
    project.key === contextProjectKey
  );
  const currentContextProject = contextProjects.find((project) =>
    projectCodeContext
      ? project.registeredWorkspace?.id === projectCodeContext.workspaceId
      : project.sessions.some((session) => session.id === currentSession?.id)
  );
  const sessionById = new Map(sessions.map((session) => [session.id, session]));
  const previousSessionId = previousReviewSessionId(
    sessionHistory,
    activeSessionId ?? undefined,
    new Set(sessionById.keys()),
  );
  const previousSession = previousSessionId
    ? sessionById.get(previousSessionId)
    : undefined;
  const linkedContext = useAnnouncedSessionContext(currentSession?.id ?? "", !!(currentSession?.continued_from || currentSession?.forked_from));
  const boundContexts = useMemo(
    () =>
      currentSession
        ? (sessionReview.bindings ?? []).map((value) => ({
          value,
          ...resolveReviewDirectory(value, currentSession, sessions, machineInventories),
        }))
        : [],
    [currentSession, machineInventories, sessionReview.bindings, sessions],
  );
  const linkedSessions = useMemo(
    () =>
      reviewLinkedSessions(currentSession, linkedContext, sessions).filter((session) =>
        !sessionReview.bindings?.includes(`session::${session.id}`)
      ),
    [currentSession, linkedContext, sessionReview.bindings, sessions],
  );
  // Worktrees list comes from the repository on screen; its registered project
  // is what a worktree of it is read under.
  const sessionProject = currentSession
    ? resolveReviewDirectory("project", currentSession, sessions, machineInventories).context
    : undefined;
  const worktreeProjectSource = projectCodeContext?.workspaceId ? projectCodeContext : sessionProject;
  const openableWorktreeProject = worktreeProjectSource?.workspaceId &&
      machineInventories.find((machine) => machine.id === worktreeProjectSource.machineId)?.local
    ? { machineId: worktreeProjectSource.machineId, workspaceId: worktreeProjectSource.workspaceId }
    : undefined;
  const viewedContext = projectCodeContext?.sessionId.startsWith("worktree::")
    ? projectCodeContext.sessionId
    : projectCodeContext
    ? projectCodeContext.workspaceId && projectCodeContext.kind !== "Worktree"
      ? `workspace::${projectCodeContext.machineId}::${projectCodeContext.workspaceId}`
      : `session::${projectCodeContext.sessionId}`
    : undefined;
  const contextReturnSession = explicitProjectContext
    ? currentSession
    : previousSession;
  useEffect(() => {
    const next = activeSessionId;
    const previous = observedSessionRef.current;
    if (!next || next === previous) return;
    if (previous && suppressedHistoryTargetRef.current !== next) {
      setSessionHistory((history) => [
        ...pushReviewSessionHistory(history, previous, next),
      ]);
    }
    if (suppressedHistoryTargetRef.current === next) {
      suppressedHistoryTargetRef.current = undefined;
    }
    observedSessionRef.current = next;
  }, [activeSessionId]);
  useEffect(() => {
    if (projectContextSessionRef.current !== activeSessionId) {
      projectContextSessionRef.current = activeSessionId;
      setProjectCodeContext(undefined);
      setReadingDefault(undefined);
    }
  }, [activeSessionId]);
  useEffect(() => {
    const valid = new Set(sessions.map((session) => session.id));
    setSessionHistory((history) => history.filter((id) => valid.has(id)));
  }, [sessions]);
  useEffect(() => {
    if (!sessionSwitcherOpen || contextTab !== "projects") return undefined;
    if (currentMachineId === "local") return undefined;
    void fetch(
      `/api/machines/${encodeURIComponent(currentMachineId)}/refresh`,
      { method: "POST" },
    ).catch(() => undefined);
    return undefined;
  }, [contextTab, currentMachineId, sessionSwitcherOpen]);
  useEffect(() => {
    if (!sessionSwitcherOpen) return undefined;
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => {
        const list = sessionListRef.current;
        if (list) list.scrollTop = 0;
      });
    });
    return () => cancelAnimationFrame(frame);
  }, [contextProjectKey, contextTab, sessionSwitcherOpen]);
  const switchSession = (
    session: SessionMeta,
    rememberCurrent = true,
  ): void => {
    navigationHaptic();
    const currentId = activeSessionId;
    if (rememberCurrent && currentId && currentId !== session.id) {
      setSessionHistory((history) => [
        ...pushReviewSessionHistory(history, currentId, session.id),
      ]);
    }
    suppressedHistoryTargetRef.current = session.id;
    setProjectCodeContext(undefined);
    setReadingDefault(undefined);
    setActiveSessionId(session.id);
    openSession(session.id);
    setSessionSwitcherOpen(false);
  };
  const openProjectTarget = (project: ReviewContextProject): void => {
    const registered = project.registeredWorkspace;
    if (!registered) return;
    navigationHaptic();
    setContextError(undefined);
    const sessionId = workspaceCodeContextId(project.machineId, registered.id);
    mutateMobileReview(sessionId, "setMode", { mode: "files" });
    setProjectCodeContext({
      ownerSessionId: boundWorkspace?.sessionId ?? null,
      sessionId,
      cwd: registered.canonicalPath,
      provider: "read-only",
      title: `Project code · ${project.label}`,
      machineId: project.machineId,
      workspaceId: registered.id,
      project: project.label,
    });
    setMode("files");
    setSourceTarget(undefined);
    setSessionSwitcherOpen(false);
  };
  const returnToPreviousSession = (): void => {
    if (!previousSession) return;
    setSessionHistory((history) => [
      ...popReviewSessionHistory(history, previousSession.id),
    ]);
    switchSession(previousSession, false);
  };
  const returnToSessionContext = (): void => {
    if (projectCodeContext && currentSession) {
      switchSession(currentSession, false);
      return;
    }
    returnToPreviousSession();
  };
  const openDefaultDirectory = (): void => {
    if (!currentSession) return;
    const resolved = resolveReviewDirectory(directoryDefault?.value, currentSession, sessions, machineInventories);
    if (!resolved.context) { setContextError(resolved.error); return; }
    setReadingDefault({ owner: currentSession.id, value: directoryDefault?.value });
    setProjectCodeContext(undefined);
    setSourceTarget(undefined);
    setSessionSwitcherOpen(false);
  };
  /** Read another code context; the Agent stays on this session. */
  const openBoundContext = (value: string): void => {
    if (!currentSession) return;
    navigationHaptic();
    const resolved = resolveReviewDirectory(value, currentSession, sessions, machineInventories);
    if (!resolved.context) { setContextError(resolved.error); return; }
    setContextError(undefined);
    setProjectCodeContext({ ...resolved.context, ownerSessionId: boundWorkspace?.sessionId ?? null });
    setMode("git");
    setSourceTarget(undefined);
    setSessionSwitcherOpen(false);
  };
  const openLinkedSession = (session: SessionMeta): void =>
    openBoundContext(`session::${session.id}`);
  const openRepositoryPull = (choice: Choice, pull: PullSummary): void => {
    const sessionId = workspace?.sessionId;
    if (!sessionId) return;
    navigationHaptic();
    pullBindRef.current?.abort();
    const controller = new AbortController();
    pullBindRef.current = controller;
    void bindPullRequest(sessionId, choice, pull.number, controller.signal)
      .then((binding) => {
        if (controller.signal.aborted) return;
        mutateMobileReview(sessionId, "setRemoteReview", { binding });
        mutateMobileReview(sessionId, "selectRemoteReview", { selected: true });
        setCloseRequest((value) => value + 1);
      }).catch(() => {
        if (controller.signal.aborted) return;
        // The listing was stale or access changed. Read it again next time,
        // and land on the Remote PR surface, which explains the connection.
        forgetRepositoryPulls(sessionId);
        mutateMobileReview(sessionId, "selectRemoteReview", { selected: true });
        setCloseRequest((value) => value + 1);
      });
  };
  const openContextSwitcher = (): void => {
    navigationHaptic();
    setContextTab("projects");
    setContextProjectKey(currentContextProject?.key);
    setSessionSwitcherOpen(true);
  };
  const activateReviewMode = (nextMode: ReviewMode): void => {
    if (nextMode === mode) return;
    navigationHaptic();
    setMode(nextMode);
    if (nextMode === "files") setCommitTarget(undefined);
    if (workspace?.sessionId) {
      mutateMobileReview(workspace.sessionId, "setMode", { mode: nextMode });
    }
  };

  if (remoteSelected && workspace?.sessionId) {
    const sessionId = workspace.sessionId;
    return <RemoteReviewApp key={sessionId} context={sessionId}
      title={currentSession?.title ?? "Session"} binding={syncedReview.remote_review}
      active={surfaceActive}
      onBind={(binding) => mutateMobileReview(sessionId, "setRemoteReview", { binding })}
      marks={{
        progress: reviewProgress,
        mark: (key, mark) => {
          setReviewProgress((current) => {
            const next = { ...current };
            if (mark === null) delete next[key];
            else next[key] = mark;
            return next;
          });
          mutateMobileReview(sessionId, "markReviewed", { key, revision: mark });
        },
      }}
      onLocal={() => mutateMobileReview(sessionId, "selectRemoteReview", { selected: false })} />;
  }

  return (
    <ReviewDrawerShell
      onOpenChange={handleDrawerOpenChange}
      closeRequest={closeRequest}
      toggleRequest={toggleDrawerRequest}
      navigationInline={navigationInline}
      inlineContainer={inlineNavigationContainer}
      onPointerDownCapture={navigationTouchGuard.onPointerDownCapture}
      onClickCapture={navigationTouchGuard.onClickCapture}
      drawer={mode === "files"
        ? (
          <ReviewFileTree
            key={workspaceIdentity ?? "none"}
            sessionId={workspace?.sessionId}
            cwd={workspace?.cwd}
            onOpenFile={openSource}
            currentPath={sourceTarget?.path}
            onClose={() => setCloseRequest((value) => value + 1)}
            refreshToken={dataRevision}
            contextLabel={contextLabel}
            embedded={navigationInline}
            actionsContainer={inlineActionsContainer}
            documentChanges={<ReviewChanges
              sessionId={workspace?.sessionId}
              documentsOnly
              compact
              onPreview={openSource}
              onOpenDiff={openDiff}
              reviewed={reviewedMarks}
              onRevision={adoptManifestRevision}
              comparison={gitComparison}
              preferredComparison={preferredGitComparison}
              onComparisonChange={changeGitComparison}
              onComparisonResolved={adoptGitComparison}
              refreshToken={dataRevision}
            />}
          />
        )
        : (
          <ReviewRepository
            key={workspaceIdentity ?? "none"}
            sessionId={workspace?.sessionId}
            machineLabel={currentMachineInventory?.display_name ?? currentMachineId}
            {...(reviewRootPath ? { projectPath: reviewRootPath } : {})}
            onOpenDiff={openDiff}
            onOpenCommit={openCommit}
            reviewed={reviewedMarks}
            onRevision={adoptManifestRevision}
            comparison={gitComparison}
            preferredComparison={preferredGitComparison}
            onComparisonChange={changeGitComparison}
            onComparisonResolved={adoptGitComparison}
            onChanges={setGitQueue}
            onClose={() => setCloseRequest((value) => value + 1)}
            refreshToken={dataRevision}
            sessions={sessions}
            machineId={currentMachineId}
            onOpenSession={switchSession}
            embedded={navigationInline}
            actionsContainer={inlineActionsContainer}
            onBrowseFiles={() => activateReviewMode("files")}
            onOpenWorktree={openableWorktreeProject
              ? (path) => openBoundContext(worktreeContextId(openableWorktreeProject.machineId, openableWorktreeProject.workspaceId, path))
              : undefined}
            linkedSession={!projectCodeContext && linkedSessions[0]
              ? { title: linkedSessions[0].title, onOpen: () => openLinkedSession(linkedSessions[0]!) }
              : undefined}
            {...(workspace?.sessionId && !projectCodeContext &&
                typeof syncedReview.remote_selected === "boolean"
              ? { onOpenPull: openRepositoryPull }
              : {})}
          />
        )}
    >
      <Stack
        sx={{
          height: "100%",
          minWidth: 0,
          bgcolor: "background.default",
          color: "text.primary",
          pt: "var(--cowboy-system-top-clearance)",
        }}
      >
        {(target.kind !== "changes" || commitTarget) && (
          <Box data-review-document-title sx={{ pl: "max(env(safe-area-inset-left, 0px), 12px)", pr: "max(env(safe-area-inset-right, 0px), 12px)", py: 0.75, flexShrink: 0, borderBottom: 1, borderColor: "divider",
            "@media (max-height: 500px) and (min-width: 600px)": { display: "flex", alignItems: "center", gap: 2, "& > :first-child": { flex: 1 }, "& > *": { minWidth: 0 } },
          }}>
            <Typography variant="caption" noWrap sx={{ display: "block", fontFamily: "var(--cowboy-font-mono)" }}>
              {commitTarget ? commitTarget.path ?? commitTarget.commit.subject : target.kind !== "changes" ? target.path : ""}
            </Typography>
            {(target.kind === "diff" || commitTarget) && <Typography variant="caption" color="text.secondary" noWrap sx={{ display: "block" }}>
              {commitTarget ? `Commit · ${commitTarget.commit.oid.slice(0, 8)}`
                : target.kind === "diff" && (target.comparison
                  ? `Compared with ${target.comparison.replace(/^refs\/(heads|remotes)\//, "")}`
                  : target.scope === "staged" ? "Staged" : target.scope === "unstaged" ? "Unstaged" : "Conflict")}
            </Typography>}
          </Box>
        )}
        {!workspace
          ? (
            <Stack
              component="main"
              alignItems="center"
              justifyContent="center"
              spacing={1.5}
              sx={{ flex: 1, px: 4, textAlign: "center" }}
            >
              <FolderOpenOutlined color="disabled" />
              <Typography color="text.secondary">
                {resolvedDirectory.error ?? "Choose a project or session to review"}
              </Typography>
              <Button onClick={openContextSwitcher}>Choose project</Button>
            </Stack>
          )
          : mode === "git" && commitTarget
          ? (
            <ReviewCommit
              sessionId={workspace.sessionId}
              commit={commitTarget.commit}
              selectedPath={commitTarget.path}
              onSelectPath={(path) =>
                setCommitTarget((current) =>
                  current
                    ? { commit: current.commit, ...(path ? { path } : {}) }
                    : current
                )}
              onFiles={setCommitPaths}
              readScrollPosition={readScrollPosition}
              rememberScrollPosition={rememberScrollPosition}
            />
          )
          : target.kind === "changes"
          ? (
            <Box
              ref={setInlineNavigationContainer}
              component="main"
              aria-label={mode === "git" ? "Repository overview" : "Workspace files"}
              sx={{ flex: 1, minHeight: 0, overflow: "hidden" }}
            />
          )
          : (
            <DocumentView
              key={`${reviewStateKey(boundWorkspace?.sessionId, workspace.sessionId)}:${target.kind}:${target.path}:${
                target.kind === "diff"
                  ? target.comparison ?? target.scope
                  : "source"
              }`}
              sessionId={workspace.sessionId}
              reviewStateId={reviewStateKey(boundWorkspace?.sessionId, workspace.sessionId)}
              onSendSelection={sendSelectionToAgent}
              onExplainSelection={explainSelection}
              bufferMode={bufferMode}
              dataRevision={dataRevision}
              active={active}
              outlineOpen={outlineOpen}
              onOutlineClose={() => setOutlineOpen(false)}
              outlineLine={syncedReview.positions?.[target.path]?.line}
              onOutlineSelect={(line) => {
                setMarkdownPreview(false);
                setSourceTarget({
                  kind: "source",
                  path: target.path,
                  revealLine: line,
                  revealRequestId: ++revealRangeId.current,
                });
              }}
              target={target}
              onRevision={setCurrentRevision}
              markdownPreview={target.kind === "source" &&
                isMarkdownReviewPath(target.path) && markdownPreview}
              languageData={languageData?.path === target.path
                ? languageData
                : undefined}
              restoreSymbol={symbolRestore}
              closeSymbolRequest={closeSymbolRequest}
              onRestoreSymbolConsumed={(id) => {
                setSymbolRestore((request) =>
                  request?.id === id ? undefined : request
                );
              }}
              onSymbolOpenChange={(point) => {
                currentSymbol.current = point;
                const frame = currentNavigationFrame.current;
                if (frame && frame.path === target.path) {
                  const { symbol: _symbol, ...frameWithoutSymbol } = frame;
                  currentNavigationFrame.current = {
                    ...frameWithoutSymbol,
                    ...(point ? { symbol: point } : {}),
                  };
                }
              }}
              onVisibleSourceLine={(line) => {
                currentVisibleLine.current = line;
                const frame = currentNavigationFrame.current;
                if (frame && frame.path === target.path) {
                  currentNavigationFrame.current = {
                    ...frame,
                    revealLine: line,
                  };
                }
              }}
              onBufferUnavailable={requestBufferRecovery}
              readScrollPosition={readScrollPosition}
              rememberScrollPosition={rememberScrollPosition}
              onMarkdownLink={followMarkdownLink}
              previewAnchor={previewAnchor}
              onNavigate={(location, origin) => {
                if (target.kind !== "source") return;
                const previous: CodeNavigationEntry = {
                  kind: "source",
                  path: target.path,
                  revealLine: currentVisibleLine.current ?? origin.row + 1,
                  symbol: origin,
                };
                setNavigationHistory((history) =>
                  [...history, previous].slice(-32)
                );
                setNavigationForwardHistory([]);
                setSymbolRestore(undefined);
                setCloseSymbolRequest((value) => value + 1);
                currentSymbol.current = undefined;
                currentNavigationFrame.current = {
                  kind: "source",
                  path: location.path,
                  revealLine: location.start.row + 1,
                  navigationRange: {
                    start: location.start,
                    end: location.end,
                  },
                };
                openSource(location.path, location.start.row + 1, true, {
                  start: location.start,
                  end: location.end,
                });
              }}
            />
          )}
        <Box
          data-mobile-backdrop-chrome="true"
          sx={{
            flexShrink: 0,
            borderTop: 1,
            borderColor: "divider",
            bgcolor: "background.default",

          }}
        >
          {!navigationInline && <ReviewTabStrip
            tabs={mode === "git" && commitTarget
              ? commitFileTabs(commitPaths)
              : modeTabs}
            activeKey={activeTabKey}
            onActivate={activateOrCollapseTab}
            beforeTouchActivate={navigationTouchGuard.arm}
            onClose={requestCloseTab}
            onCloseOthers={(key, anchor) => {
              navigationHaptic();
              setTabCloseRequest({ kind: "others", key, anchor });
            }}
            onCloseAll={(anchor) => {
              navigationHaptic();
              setTabCloseRequest({ kind: "all", anchor });
            }}
            onTogglePin={(key) => {
              const source = tabs.find((tab) =>
                tab.kind === "source" && reviewTabKey(tab) === key
              );
              if (source?.kind === "source" && workspace?.sessionId) {
                mutateMobileReview(workspace.sessionId, "setPinned", {
                  path: source.path,
                  pinned: !source.pinned,
                });
              }
              setTabs((current) => {
                if (current.some((tab) => reviewTabKey(tab) === key)) {
                  return toggleReviewTabPin(current, key);
                }
                const gitTab = gitTabs.find((tab) => reviewTabKey(tab) === key);
                return gitTab
                  ? [...current, { ...gitTab, pinned: true }]
                  : current;
              });
            }}
            allowCloseActions={mode === "files"}
            allowReorder={mode === "files"}
            allowPin={mode === "files"}
            onReorder={(movingKey, targetKey) => {
              const next = reorderReviewTabs(tabs, movingKey, targetKey);
              setTabs(next);
              if (workspace?.sessionId) {
                mutateMobileReview(workspace.sessionId, "reorder", {
                  paths: next.flatMap((tab) =>
                    tab.kind === "source" ? [tab.path] : []
                  ),
                });
              }
            }}
          />}
          <Box
            component="nav"
            aria-label="Code Review controls"
            sx={{
              pb: BOTTOM_NAV_PADDING,
              // Keep the pager's bottom baseline while leaving room for the
              // context selector and 44px actions on a narrow phone.
              pl: "max(env(safe-area-inset-left, 0px), 4px)",
              pr: "max(env(safe-area-inset-right, 0px), 4px)",
              "@media (max-height: 500px) and (min-width: 600px)": {
                display: "flex", flexWrap: "wrap", alignItems: "center",
                "& > [data-review-file-controls]": { flex: "1 0 auto" },
                "& > [data-review-workbench]": { flex: "1 0 340px", minWidth: 0 },
              },
            }}
          >
            {!navigationInline && <Toolbar data-review-file-controls
              variant="dense"
              sx={{
                minHeight: 44,
                px: 1,
                gap: 0.25,
                flexWrap: "wrap",
                "& .MuiIconButton-root": { width: 44, height: 44 },
                "@media (min-width: 600px)": { minHeight: 44 },
              }}
            >
              {mode === "files" &&
                (navigationHistory.length > 0 ||
                  navigationForwardHistory.length > 0) &&
                (
                  <Stack
                    direction="row"
                    role="group"
                    aria-label="Code navigation history"
                  >
                    <IconButton
                      aria-label="Back to previous code location"
                      disabled={navigationHistory.length === 0}
                      onClick={navigateBack}
                    >
                      <ArrowBack />
                    </IconButton>
                    <IconButton
                      aria-label="Forward to next code location"
                      disabled={navigationForwardHistory.length === 0}
                      onClick={navigateForward}
                    >
                      <ArrowForward />
                    </IconButton>
                  </Stack>
                )}
              {target.kind === "diff" && (
                <IconButton
                  aria-label={targetIsReviewed
                    ? "Mark unreviewed"
                    : "Mark reviewed"}
                  color={targetIsReviewed ? "success" : "default"}
                  disabled={!currentRevision}
                  onClick={toggleReviewed}
                >
                  {targetIsReviewed ? <CheckCircle /> : <CheckCircleOutline />}
                </IconButton>
              )}
              {target.kind === "source" &&
                  isMarkdownReviewPath(target.path) && markdownPreview
                ? (
                  // Same slot as source wrap, but its own preference: the
                  // preview defaults to sideways-scrolling code and tables.
                  <IconButton
                    aria-label={settings.markdownSoftWrap
                      ? "Scroll Markdown code and tables sideways"
                      : "Wrap Markdown code and tables"}
                    aria-pressed={settings.markdownSoftWrap}
                    color={settings.markdownSoftWrap ? "primary" : "default"}
                    onClick={() =>
                      updateReviewSettings({
                        markdownSoftWrap: !settings.markdownSoftWrap,
                      })}
                  >
                    <WrapText />
                  </IconButton>
                )
                : target.kind !== "changes" && (
                  <IconButton
                    aria-label={settings.softWrap
                      ? "Disable line wrapping"
                      : "Enable line wrapping"}
                    color={settings.softWrap ? "primary" : "default"}
                    onClick={() =>
                      updateReviewSettings({ softWrap: !settings.softWrap })}
                  >
                    <WrapText />
                  </IconButton>
                )}
              {target.kind === "source" && isMarkdownReviewPath(target.path) &&
                (
                  <IconButton
                    aria-label={markdownPreview
                      ? "Show Markdown source"
                      : "Preview Markdown"}
                    aria-pressed={markdownPreview}
                    color={markdownPreview ? "primary" : "default"}
                    onClick={() => setMarkdownPreview((value) => !value)}
                  >
                    <VisibilityOutlined />
                  </IconButton>
                )}
              {target.kind === "diff" && (
                <>
                  <Box sx={{ flex: 1 }} />
                  <IconButton
                    aria-label="Previous change"
                    disabled={reviewIndex <= 0}
                    onClick={() => moveReview(-1)}
                  >
                    <ChevronLeft />
                  </IconButton>
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ minWidth: 52, textAlign: "center" }}
                  >
                    {reviewIndex + 1} / {target.queue.length}
                  </Typography>
                  <IconButton
                    aria-label="Next change"
                    disabled={reviewIndex >= target.queue.length - 1}
                    onClick={() => moveReview(1)}
                  >
                    <ChevronRight />
                  </IconButton>
                </>
              )}
              <Box sx={{ flex: 1 }} />
              {target.kind === "source" && (
                <IconButton
                  aria-label="Open file outline"
                  aria-pressed={outlineOpen}
                  color={outlineOpen ? "primary" : "default"}
                  onClick={() => {
                    navigationHaptic();
                    setOutlineOpen(true);
                  }}
                >
                  <FormatListBulleted />
                </IconButton>
              )}
              <IconButton
                data-mobile-review-sidebar="true"
                aria-label={drawerOpen
                  ? `Close ${
                    mode === "git" ? "Git changes" : "file tree"
                  } sidebar`
                  : `Open ${
                    mode === "git" ? "Git changes" : "file tree"
                  } sidebar`}
                aria-pressed={drawerOpen}
                sx={{
                  color: "text.primary",
                  bgcolor: drawerOpen ? "action.selected" : "transparent",
                }}
                onClick={() => setToggleDrawerRequest((value) => value + 1)}
              >
                {mode === "git"
                  ? (
                    <Badge
                      variant="dot"
                      color="primary"
                      invisible={displayedChangeCount === 0}
                      slotProps={{
                        badge: {
                          "aria-label": `${displayedChangeCount} changed files`,
                        },
                      }}
                      sx={{
                        "& .MuiBadge-badge": {
                          top: 3,
                          right: 2,
                          minWidth: 7,
                          width: 7,
                          height: 7,
                        },
                      }}
                    >
                      <DifferenceOutlined />
                    </Badge>
                  )
                  : <AccountTreeOutlined />}
              </IconButton>
            </Toolbar>}
            <Stack data-review-workbench direction="row" alignItems="center" spacing={0.5}
              sx={{ px: 1, minHeight: 48, flexWrap: "wrap", "& .MuiIconButton-root": { width: 44, height: 44 } }}>
              <HintTooltip title="Agent">
                <IconButton
                  data-mobile-open-agent="true"
                  aria-label="Open Agent"
                  onClick={() => {
                    navigationHaptic();
                    openMobileProduct("agent");
                  }}
                  sx={{ width: 44, height: 44, color: "text.secondary" }}
                >
                  <ChatBubbleOutline />
                </IconButton>
              </HintTooltip>
              <ReviewContextButton
                label={projectCodeContext
              ? `Open context. Current ${contextLabel.toLowerCase()} ${projectCodeContext.project}`
              : `Switch session. Current session ${currentSession?.title ?? "none"}`}
                title={projectCodeContext?.project ?? currentSession?.title ?? currentProject ?? "Choose context"}
                statusColor={projectCodeContext ? "primary.main" : currentSession ? sessionStatusColor(currentSession.status) : "text.disabled"}
                onOpen={openContextSwitcher}
                subtitle={<>
                {projectCodeContext
                  ? `${contextLabel} · ${projectCodeContext.project}`
                  : currentProject ? `${contextLabel} · ${currentProject}`
                  : (workspace ? "Loading session…" : "Choose project or session")}
                {!navigationInline && repositoryContext?.branch
                  ? ` · ${repositoryContext.branch}`
                  : ""}
                </>}
              />
              <ReviewModeSwitcher mode={mode} onChange={activateReviewMode} />
              <WorkspaceExtensionsButton context={workspace?.sessionId} machineId={currentMachineId}
                renderTrigger={({ disabled, open }) => <ReviewMoreActions>{(close) => <>
                {navigationInline && <Box ref={setInlineActionsContainer} />}
                {contextReturnSession && (
                  <Button
                    aria-label={`Back to session ${contextReturnSession.title}`}
                    onClick={() => { close(); returnToSessionContext(); }}
                    startIcon={<Undo />}
                    sx={{ minHeight: 44, textTransform: "none" }}
                  >
                    Back to {contextReturnSession.title}
                  </Button>
                )}
                {!projectCodeContext && workspace?.sessionId &&
                  typeof syncedReview.remote_selected === "boolean" && <Button
                  aria-label={syncedReview.remote_review ? `Review remote PR #${syncedReview.remote_review.number}` : "Associate remote PR"}
                  startIcon={<CloudOutlined />}
                  sx={{ minHeight: 44, textTransform: "none" }}
                  onClick={() => {
                    close();
                    setCloseRequest((value) => value + 1);
                    mutateMobileReview(workspace.sessionId, "selectRemoteReview", { selected: true });
                  }}
                >{syncedReview.remote_review ? `PR #${syncedReview.remote_review.number}` : "Remote PR"}</Button>}
                <Button aria-label="Open extensions" disabled={disabled} onClick={() => { close(); open(); }}
                  startIcon={<ExtensionOutlined />} sx={{ minHeight: 44, textTransform: "none" }}>Extensions</Button>
              </>}</ReviewMoreActions>} />
            </Stack>
          </Box>
        </Box>
        <Popover
          open={tabCloseRequest !== undefined}
          anchorEl={tabCloseRequest?.anchor}
          onClose={() => setTabCloseRequest(undefined)}
          anchorOrigin={{ vertical: "top", horizontal: "right" }}
          transformOrigin={{ vertical: "bottom", horizontal: "right" }}
          slotProps={{
            paper: {
              sx: {
                width: "min(300px, calc(100vw - 32px))",
                borderRadius: "12px",
              },
            },
          }}
        >
          <Box data-review-tab-close-confirm sx={{ p: 1.5 }}>
            <Typography variant="body2" fontWeight={700}>
              {tabCloseTitle}
            </Typography>
            <Typography
              variant="body2"
              color="text.secondary"
              sx={{ mt: 0.5 }}
            >
              {tabCloseDetail} Files and their contents are not deleted.
            </Typography>
            <Stack
              direction="row"
              spacing={1}
              justifyContent="flex-end"
              sx={{ mt: 1.5 }}
            >
              <Button
                size="small"
                color="inherit"
                onClick={() => setTabCloseRequest(undefined)}
                sx={{ textTransform: "none" }}
              >
                Cancel
              </Button>
              <Button
                size="small"
                color="error"
                variant="contained"
                startIcon={<Close />}
                onClick={confirmTabCloseRequest}
                sx={{ textTransform: "none" }}
              >
                {tabCloseAction}
              </Button>
            </Stack>
          </Box>
        </Popover>
        <Sheet
          open={sessionSwitcherOpen}
          onClose={() => {
            setSessionSwitcherOpen(false);
          }}
          title={directoryEditing ? "Review defaults" : selectedContextProject?.label ?? "Context"}
          forceSheet
          cover
          portal
        >
          <Stack
            ref={sessionListRef}
            spacing={1.5}
            sx={{
              // DetentSheet reserves 76px + safe-area for its generic floating
              // footer. This list owns its own Close-island scroll clearance,
              // so extend through that reservation instead of rendering both
              // clearances as one empty band below the session rows.
              height:
                "calc(100dvh - 148px + 76px + env(safe-area-inset-bottom, 0px))",
              mb: "calc(-76px - env(safe-area-inset-bottom, 0px))",
              minHeight: 0,
              ...mobileNativeYScrollSx,
              pb: "calc(88px + env(safe-area-inset-bottom, 0px))",
            }}
          >
            <ReviewDirectorySetting onUseDefault={openDefaultDirectory} onEditingChange={setDirectoryEditing}
              currentDirectory={workspace ? projectCodeContext?.sessionId.startsWith("workspace::") || projectCodeContext?.sessionId.startsWith("worktree::")
                ? projectCodeContext.sessionId : `session::${workspace.sessionId}` : undefined} />
            {!directoryEditing && <>
            <Divider />
            {contextError && (
              <Alert
                severity="error"
                onClose={() => setContextError(undefined)}
              >
                {contextError}
              </Alert>
            )}
            {!selectedContextProject && (
              <SegmentedTabs
                rootProps={{ "data-mobile-context-tabs": "" }}
                aria-label="Context views"
                value={contextTab}
                onChange={setContextTab}
                sx={{ px: 0.75, boxSizing: "border-box" }}
                options={[
                  { value: "sessions", label: "Sessions" },
                  { value: "projects", label: "Projects" },
                ]}
              />
            )}
            {(boundContexts.length > 0 || viewedContext) && (
              <Stack spacing={1}>
                {boundContexts.length > 0 && (
                  <Stack spacing={0.5}>
                    <Typography
                      variant="overline"
                      color="text.secondary"
                      sx={{
                        px: 1.25,
                        fontWeight: 700,
                        letterSpacing: "0.09em",
                      }}
                    >
                      Bound repositories
                    </Typography>
                    {boundContexts.map(({ value, context, error }) => (
                      <Stack key={value} direction="row" alignItems="center">
                        <ListItemButton
                          data-review-binding={value}
                          disabled={!context}
                          selected={viewedContext === value}
                          onClick={() => openBoundContext(value)}
                          sx={{ minHeight: 44, borderRadius: 1.5, px: 1.25 }}
                        >
                          <ListItemText
                            primary={context?.title ?? value}
                            secondary={error ?? context?.cwd}
                            primaryTypographyProps={{ noWrap: true }}
                            secondaryTypographyProps={{ noWrap: true }}
                          />
                        </ListItemButton>
                        <IconButton
                          size="small"
                          aria-label={`Unbind ${context?.title ?? value}`}
                          onClick={() =>
                            mutateReviewState(currentSession!.id, "unbind", { context: value })}
                        >
                          <Close fontSize="small" />
                        </IconButton>
                      </Stack>
                    ))}
                  </Stack>
                )}
                {currentSession && viewedContext &&
                  !sessionReview.bindings?.includes(viewedContext) && (
                  <Button
                    data-review-bind-current
                    onClick={() =>
                      mutateReviewState(currentSession.id, "bind", { context: viewedContext })}
                  >
                    {`Bind ${projectCodeContext?.title ?? "this context"} to this session`}
                  </Button>
                )}
              </Stack>
            )}
            {!selectedContextProject && contextTab === "sessions" && (
              <Stack spacing={1.5}>
                {currentSession && (
                  <Stack spacing={0.5}>
                    <Typography
                      variant="overline"
                      color="text.secondary"
                      sx={{
                        px: 1.25,
                        fontWeight: 700,
                        letterSpacing: "0.09em",
                      }}
                    >
                      Current session
                    </Typography>
                    <ContextSessionRow
                      session={currentSession}
                      selected
                      onPick={() => switchSession(currentSession)}
                    />
                  </Stack>
                )}
                {linkedSessions.length > 0 && (
                  <Stack spacing={0.5}>
                    <Typography
                      variant="overline"
                      color="text.secondary"
                      sx={{
                        px: 1.25,
                        fontWeight: 700,
                        letterSpacing: "0.09em",
                      }}
                    >
                      Linked sessions
                    </Typography>
                    <Stack divider={<Divider flexItem />}>
                      {linkedSessions.map((session) => (
                        <ContextSessionRow
                          key={session.id}
                          session={session}
                          selected={projectCodeContext?.sessionId === session.id}
                          onPick={() => openLinkedSession(session)}
                        />
                      ))}
                    </Stack>
                  </Stack>
                )}
                {sessions.some((session) =>
                  session.id !== activeSessionId
                ) && (
                  <Stack spacing={0.5}>
                    <Typography
                      variant="overline"
                      color="text.secondary"
                      sx={{
                        px: 1.25,
                        fontWeight: 700,
                        letterSpacing: "0.09em",
                      }}
                    >
                      {currentSession ? "Other sessions" : "Sessions"}
                    </Typography>
                    <Stack divider={<Divider flexItem />}>
                      {sessions.filter((session) =>
                        session.id !== activeSessionId
                      ).map(
                        (session) => (
                          <ContextSessionRow
                            key={session.id}
                            session={session}
                            selected={false}
                            onPick={() => switchSession(session)}
                          />
                        ),
                      )}
                    </Stack>
                  </Stack>
                )}
              </Stack>
            )}
            {!selectedContextProject && contextTab === "projects" && (
              <Stack spacing={0.5}>
                <Typography
                  variant="overline"
                  color="text.secondary"
                  sx={{ px: 1.25, fontWeight: 700, letterSpacing: "0.09em" }}
                >
                  Registered projects
                </Typography>
                <Stack divider={<Divider flexItem />}>
                  {contextProjects.map((project) => {
                    const selected = project.key === currentContextProject?.key;
                    return (
                      <ListItemButton
                        key={project.key}
                        selected={selected}
                        onClick={() => setContextProjectKey(project.key)}
                        sx={{ minHeight: 64, px: 1.25, borderRadius: 1.5 }}
                      >
                        <FolderOutlined
                          color={selected ? "primary" : "disabled"}
                          sx={{ mr: 1.25 }}
                        />
                        <Box sx={{ flex: 1, minWidth: 0 }}>
                          <Typography variant="body2" fontWeight={650} noWrap>
                            {project.label}
                          </Typography>
                          <Typography
                            variant="caption"
                            color="text.secondary"
                            noWrap
                          >
                            {project.machineId} · {project.worktrees.length}
                            {" "}
                            worktree{project.worktrees.length === 1 ? "" : "s"}
                            {" "}
                            · {project.sessions.length}{" "}
                            session{project.sessions.length === 1 ? "" : "s"}
                          </Typography>
                          {contextProjects.some((other) => other.key !== project.key && other.label === project.label) && (
                            <Typography variant="caption" color="text.disabled" noWrap sx={{ display: "block" }}>
                              {project.registeredWorkspace?.canonicalPath ?? project.worktrees[0]?.path}
                            </Typography>
                          )}
                        </Box>
                        <ChevronRight color="disabled" fontSize="small" />
                      </ListItemButton>
                    );
                  })}
                </Stack>
              </Stack>
            )}
            {selectedContextProject && (
              <Stack spacing={1.25}>
                <ListItemButton
                  onClick={() => setContextProjectKey(undefined)}
                  sx={{
                    alignSelf: "flex-start",
                    minHeight: 44,
                    borderRadius: 2,
                  }}
                >
                  <ChevronLeft sx={{ mr: 0.75 }} />
                  <Typography variant="body2" fontWeight={650}>
                    All projects
                  </Typography>
                </ListItemButton>
                {selectedContextProject.registeredWorkspace && (
                  <ListItemButton
                    selected={projectCodeContext?.workspaceId ===
                        selectedContextProject.registeredWorkspace.id &&
                      projectCodeContext.machineId ===
                        selectedContextProject.machineId}
                    onClick={() => openProjectTarget(selectedContextProject)}
                    sx={{
                      minHeight: 64,
                      px: 1.25,
                      borderRadius: 1.5,
                    }}
                  >
                    <FolderOpenOutlined color="primary" sx={{ mr: 1.25 }} />
                    <Box sx={{ flex: 1, minWidth: 0 }}>
                      <Typography variant="body2" fontWeight={700} noWrap>
                        Project code
                      </Typography>
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        noWrap
                      >
                        Registered project checkout
                      </Typography>
                      <Typography
                        variant="caption"
                        color="text.disabled"
                        sx={{ display: "block" }}
                        noWrap
                      >
                        {selectedContextProject.registeredWorkspace
                          .canonicalPath}
                      </Typography>
                    </Box>
                    <ChevronRight color="disabled" fontSize="small" />
                  </ListItemButton>
                )}
                {selectedContextProject.worktrees.length > 0 && (
                  <Typography
                    variant="overline"
                    color="text.secondary"
                    sx={{ px: 1.25, fontWeight: 700, letterSpacing: "0.09em" }}
                  >
                    Session worktrees
                  </Typography>
                )}
                {selectedContextProject.worktrees.map((worktree) => {
                  const currentWorktree = worktree.path === workspace?.cwd;
                  const label = currentWorktree && repositoryContext?.branch
                    ? repositoryContext.branch
                    : worktree.label;
                  const onlySession = worktree.sessions.length === 1
                    ? worktree.sessions[0]
                    : undefined;
                  return (
                    <Stack key={worktree.key} spacing={0.375}>
                      <ListItemButton
                        selected={currentWorktree}
                        onClick={() => {
                          const latest = worktree.sessions[0];
                          if (latest) switchSession(latest);
                        }}
                        sx={{ minHeight: 64, px: 1.25, borderRadius: 1.5 }}
                      >
                        <AccountTreeOutlined
                          color="primary"
                          sx={{ mr: 1.25 }}
                        />
                        <Box sx={{ flex: 1, minWidth: 0 }}>
                          <Typography variant="body2" fontWeight={700} noWrap>
                            {label}
                          </Typography>
                          <Typography
                            variant="caption"
                            color="text.secondary"
                            noWrap
                          >
                            {onlySession
                              ? `Open worktree · ${onlySession.title}`
                              : `Open worktree · ${worktree.sessions.length} sessions`}
                          </Typography>
                          <Typography
                            variant="caption"
                            color="text.disabled"
                            sx={{ display: "block" }}
                            noWrap
                          >
                            {worktree.path}
                          </Typography>
                        </Box>
                        <ChevronRight color="disabled" fontSize="small" />
                      </ListItemButton>
                      {worktree.sessions.length > 1 && (
                        <Stack
                          sx={{
                            ml: 2.25,
                            borderLeft: 1,
                            borderColor: "divider",
                          }}
                          divider={<Divider flexItem />}
                        >
                          {worktree.sessions.map((session) => (
                            <ContextSessionRow
                              key={session.id}
                              session={session}
                              selected={session.id === activeSessionId}
                              showWorktree={false}
                              onPick={() => switchSession(session)}
                            />
                          ))}
                        </Stack>
                      )}
                    </Stack>
                  );
                })}
              </Stack>
            )}
            {contextTab === "sessions" && sessions.length === 0 && (
              <Typography
                color="text.secondary"
                variant="body2"
                sx={{ py: 3, textAlign: "center" }}
              >
                No sessions yet
              </Typography>
            )}
            </>}
          </Stack>
        </Sheet>
      </Stack>
    </ReviewDrawerShell>
  );
}
