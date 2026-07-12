import { alpha, Box, Typography } from "@mui/material";
import { useDesktopWorkspace, type DesktopPane } from "./DesktopWorkspaceController";

const PROMPT_MIN = 320;
const CONVERSATION_MIN = 600;

function PaneHeader({
  pane,
  children,
}: {
  pane: DesktopPane;
  children: React.ReactNode;
}): React.JSX.Element {
  const { focusedPane } = useDesktopWorkspace();
  const active = focusedPane === pane;
  return (
    <Box
      sx={{
        minHeight: 36,
        px: 2,
        display: "flex",
        alignItems: "center",
        borderBottom: 1,
        borderColor: "divider",
        flexShrink: 0,
        bgcolor: (theme) => alpha(
          active ? theme.palette.primary.main : theme.palette.background.paper,
          active ? 0.08 : 0.16,
        ),
        boxShadow: active ? "inset 0 2px 0 0 currentColor" : "none",
        color: active ? "primary.main" : "text.secondary",
      }}
    >
      <Typography
        variant="overline"
        color="inherit"
        sx={{ fontWeight: 700, letterSpacing: "0.09em", lineHeight: 1 }}
      >
        {children}
      </Typography>
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
        sx={{
          width:
            `min(${String(promptWidth)}px, 45%, max(${String(PROMPT_MIN)}px, calc(100% - ${String(CONVERSATION_MIN)}px)))`,
          flexShrink: 0,
          minWidth: 0,
          display: "flex",
          flexDirection: "column",
          minHeight: 0,
          bgcolor: (theme) => alpha(theme.palette.background.paper, 0.24),
        }}
      >
        <PaneHeader pane="prompt">Prompt</PaneHeader>
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
        sx={{
          flex: 1,
          minWidth: 0,
          position: "relative",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <PaneHeader pane="conversation">Conversation</PaneHeader>
        {conversation}
      </Box>
    </Box>
  );
}
