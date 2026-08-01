import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import { ArrowBack, MenuOpen } from "@mui/icons-material";
import { useMemo } from "react";
import {
  type DesktopPane,
  useDesktopWorkspace,
} from "./DesktopWorkspaceController";
import { DesktopRegionShortcut } from "./DesktopRegionShortcut";
import { DesktopConversationControls } from "./DesktopConversationControls";
import { MOD_LABEL } from "../platform";
import type { TranscriptProjection } from "../explore/exploreStore";
import { DesktopProjectionToggle } from "../explore/ProjectionToggle";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./commands/DesktopCommandProvider";
import { DESKTOP_FOCUS_PROMPT_SHORTCUT } from "./commands/workspaceShortcuts";

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
        sx={{ fontWeight: 700, letterSpacing: "0.09em", lineHeight: 1 }}
      >
        {children}
      </Typography>
      <Box sx={{ flex: 1 }} />
      {actions}
      <DesktopRegionShortcut
        shortcut={shortcut.value}
        title={shortcut.title}
        singleKeycap={`${MOD_LABEL}${shortcut.value.slice(-1)}`}
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
          <Typography variant="caption" color="text.secondary">
            {projection === "explore" ? "Question pages" : "Conversation"}
          </Typography>
          <Box sx={{ flex: 1 }} />
          {projection === "explore" && (
            <Button
              size="small"
              color="inherit"
              startIcon={<MenuOpen fontSize="small" />}
              onClick={(): void => {
                const closing = workspace.readingSidebarOpen;
                workspace.setReadingSidebarOpen(!closing);
                if (closing) {
                  requestAnimationFrame(() =>
                    workspace.focusRegion("conversation.transcript"));
                }
              }}
            >
              {workspace.readingSidebarOpen ? "Hide pages" : "Show pages"}
              <Typography component="span" variant="caption" sx={{ ml: 1, opacity: 0.55 }}>
                P
              </Typography>
            </Button>
          )}
          <Button
            size="small"
            color="inherit"
            startIcon={<ArrowBack fontSize="small" />}
            onClick={(): void => workspace.setProductMode("agent")}
          >
            Agent
            <Typography component="span" variant="caption" sx={{ ml: 1, opacity: 0.55 }}>
              Esc
            </Typography>
          </Button>
        </Stack>
        <Box
          data-desktop-pane="conversation"
          data-desktop-pane-focused="true"
          sx={{ flex: 1, minHeight: 0, display: "flex" }}
        >
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
          // Prefer the persisted working width, but protect both productive
          // surfaces. The conversation floor is deliberately lower than before:
          // at a compact Desktop width the Sessions rail collapses first, then
          // Prompt and Conversation share the full window instead of falling
          // back to Mobile's vertical stack.
          width:
            `min(${String(promptWidth)}px, 46%, max(${String(PROMPT_MIN)}px, calc(100% - ${String(CONVERSATION_MIN)}px)))`,
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
        onPointerDown={onResizeStart}
        sx={{
          flex: "0 0 auto",
          alignSelf: "stretch",
          width: "1px",
          bgcolor: resizing ? "primary.main" : "divider",
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
          shortcut={{ value: "Mod+L", title: "Focus Conversation" }}
          actions={(
            <>
              <DesktopProjectionToggle
                projection={projection}
                onChange={onProjectionChange}
                shortcutActive={conversationShortcutsActive}
              />
              <DesktopConversationControls
                sessionId={sessionId}
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
