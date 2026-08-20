import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import { ArrowBack, ListAltOutlined } from "@mui/icons-material";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  type DesktopPane,
  useDesktopWorkspace,
} from "./DesktopWorkspaceController";
import { DesktopRegionShortcut } from "./DesktopRegionShortcut";
import { DesktopConversationControls } from "./DesktopConversationControls";
import { DesktopReadingModeControl } from "./DesktopReadingModeControl";
import { desktopEmbeddedControlSx } from "./DesktopEmbeddedControl";
import { ShortcutKeycap } from "../ShortcutKeycap";
import type { TranscriptProjection } from "../explore/exploreStore";
import { DesktopReadingQuestionDirectory } from "../explore/ExploreSurface";
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
  clampReadingQuestionsWidth,
  COMPOSER_COL_MAX,
  COMPOSER_COL_MIN,
  readingQuestionsWidthStore,
  READING_QUESTIONS_MAX,
  READING_QUESTIONS_MIN,
} from "../desktopLayout";
import {
  DESKTOP_SPLITTER_ADJUST_EVENT,
  splitterAdjustment,
} from "./desktopSplitterKeyboard";

const PROMPT_MIN = 360;
const CONVERSATION_MIN = 520;

function PaneHeader({
  pane,
  shortcut,
  actions,
  children,
}: {
  pane: DesktopPane;
  shortcut: { value: string; title: string };
  actions?: React.ReactNode;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <Box
      data-desktop-pane-header
      data-pane={pane}
      sx={{
        minHeight: 36,
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
        : <Box sx={{ flex: 1 }} />}
      <DesktopRegionShortcut
        shortcut={shortcut.value}
        title={shortcut.title}
        singleKeycap={shortcut.value}
        sx={{ ml: 0.5 }}
      />
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
}: {
  promptWidth: number;
  resizing: boolean;
  onResizeStart: (event: React.PointerEvent<HTMLDivElement>) => void;
  prompt: React.ReactNode;
  conversation: React.ReactNode;
  sessionId: string;
  projection: TranscriptProjection;
  onProjectionChange: (projection: TranscriptProjection) => void;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const [questionsWidth, setQuestionsWidth] = useState(
    readingQuestionsWidthStore.get,
  );
  const [questionsResizing, setQuestionsResizing] = useState(false);
  const questionsWidthRef = useRef(questionsWidth);
  questionsWidthRef.current = questionsWidth;
  useEffect(() => {
    const onKeyboardResize = (event: Event): void => {
      const adjustment = splitterAdjustment(event);
      if (adjustment?.splitter !== "questions-page") return;
      setQuestionsWidth((current) => {
        const next = clampReadingQuestionsWidth(current + adjustment.delta);
        questionsWidthRef.current = next;
        readingQuestionsWidthStore.set(next);
        return next;
      });
    };
    globalThis.addEventListener(DESKTOP_SPLITTER_ADJUST_EVENT, onKeyboardResize);
    return (): void =>
      globalThis.removeEventListener(
        DESKTOP_SPLITTER_ADJUST_EVENT,
        onKeyboardResize,
      );
  }, []);
  useEffect(() => {
    if (!questionsResizing) return undefined;
    const previousCursor = document.body.style.cursor;
    const previousSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    return (): void => {
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousSelect;
    };
  }, [questionsResizing]);
  function startQuestionsResize(event: React.PointerEvent<HTMLDivElement>): void {
    if (event.button !== 0) return;
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = questionsWidthRef.current;
    const element = event.currentTarget;
    element.setPointerCapture(event.pointerId);
    setQuestionsResizing(true);
    const onMove = (moveEvent: PointerEvent): void => {
      setQuestionsWidth(clampReadingQuestionsWidth(
        startWidth + (moveEvent.clientX - startX),
      ));
    };
    const onUp = (): void => {
      element.releasePointerCapture(event.pointerId);
      element.removeEventListener("pointermove", onMove);
      element.removeEventListener("pointerup", onUp);
      setQuestionsResizing(false);
      readingQuestionsWidthStore.set(questionsWidthRef.current);
    };
    element.addEventListener("pointermove", onMove);
    element.addEventListener("pointerup", onUp);
  }
  const conversationShortcutsActive = workspace.focusedPane === "conversation";
  const projectionPageName = workspace.productMode === "reading" ? "Page" : "Explore";
  const toggleProjectionCommand = useMemo<DesktopCommand>(() => ({
    id: "conversation.toggleProjection",
    title: `Switch to ${projection === "history" ? projectionPageName : "History"}`,
    description: `Toggle the Conversation between History and ${projectionPageName}`,
    group: "Conversation",
    shortcut: "V",
    contexts: ["conversation"],
    run: () => onProjectionChange(
      projection === "history" ? "explore" : "history",
    ),
  }), [onProjectionChange, projection, projectionPageName]);
  useDesktopCommand(toggleProjectionCommand);

  if (workspace.productMode === "reading") {
    return (
      <Box
        data-desktop-product-mode="reading"
        sx={{
          position: "fixed",
          inset: 0,
          zIndex: (theme) => theme.zIndex.modal - 1,
          display: "flex",
          flexDirection: "column",
          bgcolor: "background.default",
        }}
      >
        <Stack
          component="header"
          direction="row"
          alignItems="center"
          spacing={1}
          sx={{
            minHeight: 48,
            px: 2,
            borderBottom: 1,
            borderColor: "divider",
            bgcolor: (theme) => alpha(theme.palette.background.paper, 0.72),
          }}
        >
          <Typography variant="overline" sx={{ fontWeight: 750, letterSpacing: "0.1em" }}>
            Reading
          </Typography>
          <DesktopProjectionToggle
            projection={projection}
            pageLabel="Page"
            onChange={onProjectionChange}
            shortcutActive
          />
          <Button
            aria-pressed={workspace.readingSidebarOpen}
            size="small"
            color={workspace.readingSidebarOpen ? "primary" : "inherit"}
            variant="outlined"
            startIcon={<ListAltOutlined fontSize="small" />}
            onClick={(): void => {
              const closing = workspace.readingSidebarOpen;
              workspace.setReadingSidebarOpen(!closing);
              if (closing) {
                requestAnimationFrame(() =>
                  workspace.focusRegion("conversation.transcript"));
              }
            }}
            sx={{
              ...desktopEmbeddedControlSx({ active: true, open: workspace.readingSidebarOpen }),
              height: 34,
              px: 0.9,
              gap: 0.65,
              textTransform: "none",
              "& .MuiButton-startIcon": { mr: 0 },
            }}
          >
            Pages
            <ShortcutKeycap keyLabel="P" variant="global" accent sx={{ ml: 0.15 }} />
          </Button>
          <Box sx={{ flex: 1 }} />
          <DesktopConversationControls
            sessionId={sessionId}
            projection={projection}
            shortcutActive
          />
          <Button
            size="small"
            color="inherit"
            variant="outlined"
            startIcon={<ArrowBack fontSize="small" />}
            onClick={(): void => workspace.setProductMode("agent")}
            sx={{
              ...desktopEmbeddedControlSx({ active: true }),
              height: 34,
              px: 0.9,
              gap: 0.65,
              textTransform: "none",
              "& .MuiButton-startIcon": { mr: 0 },
            }}
          >
            Agent
            <ShortcutKeycap keyLabel="Esc" variant="global" accent sx={{ ml: 0.15 }} />
          </Button>
        </Stack>
        <Box
          data-desktop-pane="conversation"
          data-desktop-pane-focused="true"
          sx={{ flex: 1, minHeight: 0, display: "flex" }}
        >
          {workspace.readingSidebarOpen && (
            <DesktopReadingQuestionDirectory
              sessionId={sessionId}
              projection={projection}
              width={questionsWidth}
              onClose={(): void => {
                workspace.setReadingSidebarOpen(false);
                requestAnimationFrame(() =>
                  workspace.focusRegion("conversation.transcript"));
              }}
            />
          )}
          {workspace.readingSidebarOpen && (
            <Box
              role="separator"
              aria-orientation="vertical"
              aria-label="Resize page index"
              title={`Resize layout · ${DESKTOP_SHORTCUTS.resize}`}
              aria-valuemin={READING_QUESTIONS_MIN}
              aria-valuemax={READING_QUESTIONS_MAX}
              aria-valuenow={Math.round(questionsWidth)}
              data-desktop-splitter="questions-page"
              data-desktop-splitter-selected={
                workspace.selectedSplitter === "questions-page" ? "true" : undefined
              }
              tabIndex={-1}
              onPointerDown={startQuestionsResize}
              sx={{
                flex: "0 0 auto",
                alignSelf: "stretch",
                width: "1px",
                bgcolor: questionsResizing ||
                    workspace.selectedSplitter === "questions-page"
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
              {workspace.selectedSplitter === "questions-page" && (
                <DesktopSplitterHint />
              )}
            </Box>
          )}
          <Box
            data-desktop-region="conversation.transcript"
            data-desktop-navigation="scroll"
            data-desktop-focused="true"
            tabIndex={-1}
            sx={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex" }}
          >
            {conversation}
          </Box>
        </Box>
      </Box>
    );
  }

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
      <Box
        component="section"
        aria-label="Prompt pane"
        data-desktop-pane="prompt"
        tabIndex={-1}
        sx={{
          // Prefer the persisted working width while preserving Conversation's
          // productive floor. Do not impose a second percentage ceiling: it
          // would let pointer/keyboard state change while the visible divider
          // stayed fixed, making Resize mode appear broken.
          width:
            `min(${String(promptWidth)}px, max(${String(PROMPT_MIN)}px, calc(100% - ${String(CONVERSATION_MIN)}px)))`,
          flexShrink: 0,
          minWidth: 0,
          display: "flex",
          flexDirection: "column",
          minHeight: 0,
          bgcolor: (theme) => alpha(theme.palette.background.paper, 0.24),
        }}
      >
        <PaneHeader
          pane="prompt"
          shortcut={{
            value: DESKTOP_FOCUS_PROMPT_SHORTCUT,
            title: "Focus Message the agent",
          }}
        >
          Prompt
        </PaneHeader>
        {prompt}
      </Box>

      <Box
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize composer column"
        title={`Resize layout · ${DESKTOP_SHORTCUTS.resize}`}
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

      <Box
        component="section"
        aria-label="Conversation pane"
        data-desktop-pane="conversation"
        tabIndex={-1}
        sx={{
          flex: 1,
          minWidth: 0,
          position: "relative",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <PaneHeader
          pane="conversation"
          shortcut={{ value: DESKTOP_SHORTCUTS.focusConversation, title: "Focus Conversation" }}
          actions={(
            <>
              <DesktopProjectionToggle
                projection={projection}
                onChange={onProjectionChange}
                shortcutActive={conversationShortcutsActive}
              />
              <DesktopReadingModeControl
                shortcutActive={conversationShortcutsActive}
                onEnter={(): void => {
                  workspace.setProductMode("reading");
                  requestAnimationFrame(() =>
                    workspace.focusRegion("conversation.transcript"));
                }}
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
          sx={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}
        >
          {conversation}
        </Box>
      </Box>
    </Box>
  );
}
