import { alpha, Box, Typography } from "@mui/material";
import type { DesktopPane } from "./DesktopWorkspaceController";
import { DesktopRegionShortcut } from "./DesktopRegionShortcut";

const PROMPT_MIN = 360;
const CONVERSATION_MIN = 520;

function PaneHeader({
  pane,
  shortcut,
  children,
}: {
  pane: DesktopPane;
  shortcut: { key: string; title: string };
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
      <DesktopRegionShortcut shortcutKey={shortcut.key} title={shortcut.title} />
    </Box>
  );
}

export function DesktopWorkspace({
  promptWidth,
  resizing,
  onResizeStart,
  prompt,
  conversation,
}: {
  promptWidth: number;
  resizing: boolean;
  onResizeStart: (event: React.PointerEvent<HTMLDivElement>) => void;
  prompt: React.ReactNode;
  conversation: React.ReactNode;
}): React.JSX.Element {
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
        <PaneHeader pane="prompt" shortcut={{ key: "E", title: "Focus prompt editor" }}>
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
          shortcut={{ key: "C", title: "Focus conversation" }}
        >
          Conversation
        </PaneHeader>
        <Box
          data-desktop-region="conversation.transcript"
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
