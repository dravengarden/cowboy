import { desktopSize } from "../surface/desktopSize";
import { alpha, Box, Typography } from "@mui/material";
import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { DifferenceOutlined } from "@mui/icons-material";
import { IconButton } from "@mui/material";
import {
  type DesktopPane,
  useDesktopWorkspace,
} from "./DesktopWorkspaceController";
import { DesktopConnectionNotice } from "./DesktopConnectionNotice";
import { DesktopRegionShortcut } from "./DesktopRegionShortcut";
import { DesktopConversationControls } from "./DesktopConversationControls";
import { DesktopApprovalDock } from "./DesktopApprovalDock";
import { HintTooltip } from "../HintTooltip";
import type { TranscriptProjection } from "../explore/exploreStore";
import { DesktopProjectionToggle } from "../explore/ProjectionToggle";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./commands/DesktopCommandProvider";
import {
  DESKTOP_FOCUS_PROMPT_SHORTCUT,
  DESKTOP_SHORTCUTS,
} from "./commands/workspaceShortcuts";
import { DesktopSplitterHint } from "./DesktopSplitterHint";
import {
  COMPOSER_COL_MAX,
  COMPOSER_COL_MIN,
  DESKTOP_CONVERSATION_MIN,
  DESKTOP_PROMPT_MIN,
} from "../desktopLayout";
import {
  DesktopCollapsedPaneRail,
  DesktopPaneCollapseButton,
  DesktopRailCount,
} from "./DesktopPaneCollapse";
import { useStoreSelector } from "../store";
import { desktopLeaderSequence, DESKTOP_WORKSPACE_KEYS } from "./commands/workspaceShortcuts";
import { MOBILE_OPEN_PRODUCT_EVENT, mobileProductFromEvent } from "../mobile/appPagerMotion";
import { focusedSubagentStop, SUBAGENT_STOP_KEY } from "../subagents";

const ReviewApp = lazy(async () => {
  const module = await import("../mobile/review/ReviewApp");
  return { default: module.ReviewApp };
});
const ignoreReviewDrawer = (): void => {};

const PROMPT_MIN = DESKTOP_PROMPT_MIN;
const CONVERSATION_MIN = DESKTOP_CONVERSATION_MIN;
// A Prompt that inherits the Conversation's width stays a readable writing
// column instead of a full-window line length.
const NO_MESSAGES: readonly unknown[] = [];

/** Queue and draft counts stay visible on the collapsed Prompt rail. */
function PromptRailCounts({ sessionId }: { sessionId: string }): React.JSX.Element {
  const queued = useStoreSelector((state) =>
    (state.queues.get(sessionId) ?? NO_MESSAGES).length
  );
  const drafts = useStoreSelector((state) =>
    (state.drafts.get(sessionId) ?? NO_MESSAGES).length
  );
  return (
    <>
      <DesktopRailCount count={queued} label="queued" />
      <DesktopRailCount count={drafts} label={drafts === 1 ? "draft" : "drafts"} />
    </>
  );
}

function PaneHeader({
  pane,
  shortcut,
  actions,
  status,
  collapsible = false,
  children,
}: {
  pane: DesktopPane;
  shortcut: { value: string; title: string };
  actions?: React.ReactNode;
  /** Transient state shown in the fixed-height header so it never shifts the
   * pane body (the connection notice). */
  status?: React.ReactNode;
  collapsible?: boolean;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <Box
      data-desktop-pane-header
      data-pane={pane}
      sx={{
        minHeight: desktopSize(36),
        px: 2,
        display: "flex",
        alignItems: "center",
        borderBottom: 1,
        borderColor: "divider",
        flexShrink: 0,
      }}
    >
      <Typography
        variant="overline"
        color="inherit"
        sx={{
          fontWeight: 700,
          letterSpacing: "0.09em",
          lineHeight: 1,
          flexShrink: 0,
        }}
      >
        {children}
      </Typography>
      {actions
        ? (
          <Box
            data-desktop-pane-action-rail
            role="toolbar"
            aria-label={`${pane} pane actions`}
            sx={{
              flex: "1 1 auto",
              minWidth: 0,
              ml: 1,
              overflowX: "auto",
              overflowY: "hidden",
              overscrollBehaviorX: "contain",
              WebkitOverflowScrolling: "touch",
              scrollbarWidth: "thin",
              "&::-webkit-scrollbar": { height: 4 },
              "&::-webkit-scrollbar-thumb": {
                bgcolor: "action.disabled",
                borderRadius: 99,
              },
            }}
          >
            <Box
              data-desktop-pane-action-track
              sx={{
                width: "max-content",
                minWidth: "100%",
                display: "flex",
                alignItems: "center",
                justifyContent: "flex-end",
                "& > *": { flexShrink: 0 },
              }}
            >
              {actions}
            </Box>
          </Box>
        )
        : (
          <Box
            sx={{
              flex: 1,
              minWidth: 0,
              mx: 1,
              display: "flex",
              justifyContent: "flex-end",
            }}
          >
            {status}
          </Box>
        )}
      <DesktopRegionShortcut
        shortcut={shortcut.value}
        title={shortcut.title}
        singleKeycap={shortcut.value}
        sx={{ ml: 0.5 }}
      />
      {collapsible && <DesktopPaneCollapseButton pane={pane} sx={{ ml: 0.75, mr: -0.75 }} />}
    </Box>
  );
}

export function DesktopWorkspace({
  promptWidth,
  resizing,
  onResizeStart,
  prompt,
  conversation,
  sessionId,
  projection,
  onProjectionChange,
  conversationActivity,
  collapseIntent = null,
  promptRatio = null,
}: {
  promptWidth: number;
  resizing: boolean;
  onResizeStart: (event: React.PointerEvent<HTMLDivElement>) => void;
  prompt: React.ReactNode;
  conversation: React.ReactNode;
  sessionId: string;
  projection: TranscriptProjection;
  onProjectionChange: (projection: TranscriptProjection) => void;
  /** Live agent status shown on the collapsed Conversation rail. */
  conversationActivity?: React.ReactNode;
  /** Pane a splitter drag will collapse on release; previewed by dimming. */
  collapseIntent?: "prompt" | "conversation" | null;
  /** Share of the workspace for Prompt while Sessions is collapsed (the two
   *  work panes split evenly by default); `null` uses `promptWidth`. */
  promptRatio?: number | null;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const promptCollapsed = workspace.collapsedPanes.prompt;
  const conversationCollapsed = workspace.collapsedPanes.conversation;
  const conversationShortcutsActive = workspace.focusedPane === "conversation";
  const toggleProjectionCommand = useMemo<DesktopCommand>(() => ({
    id: "conversation.toggleProjection",
    title: `Switch to ${projection === "history" ? "Explore" : "History"}`,
    description: "Toggle the Conversation between History and Explore",
    group: "Conversation",
    shortcut: "V",
    contexts: ["conversation"],
    run: () => onProjectionChange(
      projection === "history" ? "explore" : "history",
    ),
  }), [onProjectionChange, projection]);
  useDesktopCommand(toggleProjectionCommand);
  // A subagent row's Stop, from the keyboard: Tab selects the row's widget,
  // the key opens the row's own Stop confirmation.
  const stopSubagentCommand = useMemo<DesktopCommand>(() => ({
    id: "conversation.stopSubagent",
    title: "Stop the focused subagent",
    description: "Ask to stop the subagent whose row has focus",
    group: "Conversation",
    shortcut: SUBAGENT_STOP_KEY,
    contexts: ["conversation"],
    regions: ["conversation.transcript"],
    when: () => focusedSubagentStop() !== null,
    disabledReason: "Focus a subagent row that can be stopped",
    run: () => focusedSubagentStop()?.click(),
  }), []);
  useDesktopCommand(stopSubagentCommand);
  // Review takes the Conversation pane and leaves the transcript mounted, so
  // returning keeps its reading position; the Prompt stays beside the code.
  const [reviewSession, setReviewSession] = useState<string>();
  const reviewOpen = reviewSession === sessionId;
  const toggleReviewCommand = useMemo<DesktopCommand>(() => ({
    id: "conversation.toggleReview",
    title: reviewOpen ? "Return to the Conversation" : "Review code",
    description: "Show this session's changes in the Conversation pane",
    group: "Conversation",
    sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.review),
    run: () => setReviewSession(reviewOpen ? undefined : sessionId),
  }), [reviewOpen, sessionId]);
  useDesktopCommand(toggleReviewCommand);
  useEffect(() => {
    // Review's Agent action and Send to Agent, which mean the same here.
    const close = (event: Event): void => {
      if (mobileProductFromEvent(event) === "agent") setReviewSession(undefined);
    };
    globalThis.addEventListener(MOBILE_OPEN_PRODUCT_EVENT, close);
    return () => globalThis.removeEventListener(MOBILE_OPEN_PRODUCT_EVENT, close);
  }, []);

  return (
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
      {promptCollapsed && (
        <DesktopCollapsedPaneRail
          pane="prompt"
          side="left"
          indicators={<PromptRailCounts sessionId={sessionId} />}
        />
      )}
      <Box
        component="section"
        aria-label="Prompt pane"
        data-desktop-pane="prompt"
        data-desktop-pane-collapsed={promptCollapsed ? "true" : undefined}
        tabIndex={-1}
        sx={{
          // Prefer the persisted working width while preserving Conversation's
          // productive floor. Do not impose a second percentage ceiling: it
          // would let pointer/keyboard state change while the visible divider
          // stayed fixed, making Resize mode appear broken.
          width: conversationCollapsed
            ? "auto"
            : promptRatio !== null
            ? `clamp(${String(PROMPT_MIN)}px, ${String(promptRatio * 100)}%, max(${String(PROMPT_MIN)}px, calc(100% - ${String(CONVERSATION_MIN)}px)))`
            : `min(${String(promptWidth)}px, max(${String(PROMPT_MIN)}px, calc(100% - ${String(CONVERSATION_MIN)}px)))`,
          flex: conversationCollapsed ? 1 : undefined,
          flexShrink: 0,
          minWidth: 0,
          // Collapsed panes stay mounted so the editor keeps its draft, undo
          // history, Vim mode and IME state; they are simply not rendered.
          display: promptCollapsed ? "none" : "flex",
          flexDirection: "column",
          minHeight: 0,
          bgcolor: (theme) => alpha(theme.palette.background.paper, 0.24),
          opacity: collapseIntent === "prompt" ? 0.38 : 1,
          transition: "opacity 120ms ease",
        }}
      >
        <PaneHeader
          pane="prompt"
          status={<DesktopConnectionNotice />}
          collapsible
          shortcut={{
            value: DESKTOP_FOCUS_PROMPT_SHORTCUT,
            title: "Focus Message the agent",
          }}
        >
          Prompt
        </PaneHeader>
        <Box
          sx={{
            flex: 1,
            minHeight: 0,
            width: "100%",
            alignSelf: "center",
            display: "flex",
            flexDirection: "column",
          }}
        >
          {prompt}
        </Box>
      </Box>

      {!promptCollapsed && !conversationCollapsed && (
        <HintTooltip
          title="Resize layout"
          shortcut={DESKTOP_SHORTCUTS.resize}
          detail="Drag past the edge to collapse"
          followCursor
          // A pointer hint: selecting the splitter by keyboard shows
          // its own Resize hint instead.
          disableFocusListener
          placement="right"
          suppressed={resizing ||
            workspace.selectedSplitter === "prompt-conversation"}
        >
          <Box
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize composer column"
            aria-valuemin={COMPOSER_COL_MIN}
            aria-valuemax={COMPOSER_COL_MAX}
            aria-valuenow={Math.round(promptWidth)}
            data-desktop-splitter="prompt-conversation"
            data-desktop-splitter-selected={
              workspace.selectedSplitter === "prompt-conversation" ? "true" : undefined
            }
            tabIndex={-1}
            onPointerDown={onResizeStart}
            sx={{
              flex: "0 0 auto",
              alignSelf: "stretch",
              width: "1px",
              bgcolor: resizing || workspace.selectedSplitter === "prompt-conversation"
                ? "primary.main"
                : "divider",
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
              "&:focus": { outline: "none" },
            }}
          >
            {workspace.selectedSplitter === "prompt-conversation" && (
              <DesktopSplitterHint />
            )}
          </Box>
        </HintTooltip>
      )}

      <Box
        component="section"
        aria-label="Conversation pane"
        data-desktop-pane="conversation"
        data-desktop-pane-collapsed={conversationCollapsed ? "true" : undefined}
        tabIndex={-1}
        sx={{
          flex: 1,
          minWidth: 0,
          position: "relative",
          display: conversationCollapsed ? "none" : "flex",
          flexDirection: "column",
          opacity: collapseIntent === "conversation" ? 0.38 : 1,
          transition: "opacity 120ms ease",
        }}
      >
        <PaneHeader
          pane="conversation"
          collapsible
          shortcut={{ value: DESKTOP_SHORTCUTS.focusConversation, title: "Focus Conversation" }}
          actions={(
            <>
              <HintTooltip title={reviewOpen ? "Return to the Conversation" : "Review code"}>
                <IconButton
                  data-desktop-review-toggle
                  size="small"
                  aria-label={reviewOpen ? "Return to the Conversation" : "Review code"}
                  aria-pressed={reviewOpen}
                  color={reviewOpen ? "primary" : "default"}
                  onClick={toggleReviewCommand.run}
                >
                  <DifferenceOutlined fontSize="small" />
                </IconButton>
              </HintTooltip>
              <DesktopProjectionToggle
                projection={projection}
                onChange={onProjectionChange}
                shortcutActive={conversationShortcutsActive}
              />
              <DesktopConversationControls
                sessionId={sessionId}
                projection={projection}
                shortcutActive={conversationShortcutsActive}
              />
            </>
          )}
        >
          Conversation
        </PaneHeader>
        <Box
          data-desktop-region="conversation.transcript"
          data-desktop-navigation="scroll"
          data-desktop-focus-default
          tabIndex={-1}
          sx={{
            flex: 1,
            minHeight: 0,
            display: reviewOpen ? "none" : "flex",
            flexDirection: "column",
          }}
        >
          {conversation}
          {
            /* A folded pane shows nothing, so the Composer keeps the answer
              (approvalDockStore). */
          }
          {!conversationCollapsed && <DesktopApprovalDock sessionId={sessionId} />}
        </Box>
        {reviewOpen && (
          <Box
            data-desktop-review
            sx={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}
          >
            <Suspense fallback={null}>
              <ReviewApp active onDrawerOpenChange={ignoreReviewDrawer} />
            </Suspense>
          </Box>
        )}
      </Box>
      {conversationCollapsed && (
        <DesktopCollapsedPaneRail
          pane="conversation"
          side="right"
          indicators={conversationActivity}
        />
      )}
    </Box>
  );
}
