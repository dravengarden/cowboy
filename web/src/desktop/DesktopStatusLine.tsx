import { alpha, Box, ButtonBase, Divider, Stack, Tooltip } from "@mui/material";
import { useExploreSessionState } from "../explore/exploreStore";
import type { Status } from "../protocol";
import { useStoreSelector } from "../store";
import { useVimMode, VIM_MODE_COLOR } from "../vimModeStore";
import { useVimSetting } from "../vimSetting";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopKeycap, DesktopShortcut } from "./commands/DesktopKeycap";
import { useDesktopCommands } from "./commands/DesktopCommandProvider";
import { useImeStatus } from "./vim/imeStatusStore";
import { useVimMacroRecording } from "./vim/macroStatusStore";
import {
  DESKTOP_FOCUS_PLAN_SHORTCUT,
  DESKTOP_FOCUS_PROMPT_SHORTCUT,
} from "./commands/workspaceShortcuts";

function Segment({
  label,
  tooltip,
  color = "text.secondary",
  mono = false,
  onClick,
}: {
  label: React.ReactNode;
  tooltip?: string;
  color?: string;
  mono?: boolean;
  onClick?: () => void;
}): React.JSX.Element {
  const body = (
    <ButtonBase
      disableRipple
      tabIndex={-1}
      onClick={onClick}
      sx={{
        height: 28,
        px: 1,
        color,
        fontSize: "0.6875rem",
        fontWeight: 650,
        letterSpacing: "0.035em",
        whiteSpace: "nowrap",
        fontFamily: mono && typeof label === "string" ? "monospace" : "inherit",
        "&:hover": { bgcolor: "action.hover", color: "text.primary" },
      }}
    >
      {label}
    </ButtonBase>
  );
  return tooltip ? <Tooltip title={tooltip}>{body}</Tooltip> : body;
}

interface RegionHint {
  keys: string;
  label: string;
}

const HISTORY_HINTS: RegionHint[] = [
  { keys: "J/K", label: "Scroll" },
  { keys: "Ctrl+D/U", label: "Half page" },
  { keys: "Ctrl+F/B", label: "Page" },
  { keys: "GG/G", label: "Oldest/latest" },
  { keys: "V", label: "Explore" },
  { keys: "F", label: "Follow" },
  { keys: "Tab", label: "Widget" },
  { keys: "H/L", label: "Close/open" },
];

const EXPLORE_HINTS: RegionHint[] = [
  { keys: "J/K", label: "Page" },
  { keys: "Ctrl+D/U", label: "Half page" },
  { keys: "Ctrl+F/B", label: "Scroll page" },
  { keys: "GG/G", label: "Oldest/latest" },
  { keys: "P", label: "Pages" },
  { keys: "N", label: "New question" },
  { keys: "V", label: "History" },
  { keys: "F", label: "Follow" },
  { keys: "Tab", label: "Widget" },
  { keys: "H/L", label: "Close/open" },
];

function regionHints(
  region: string | null,
  status: Status,
  projection: "history" | "explore",
): RegionHint[] {
  switch (region) {
    case "topbar.controls":
      return [
        { keys: "H/L", label: "Select" },
        { keys: "Enter", label: "Open" },
        { keys: "R", label: "Config" },
        { keys: "U", label: "Usage" },
        { keys: "L", label: "Reload" },
        { keys: "C", label: "Compact" },
        { keys: "X", label: "Clear" },
        ...(status === "busy" ? [{ keys: "S", label: "Stop" }] : []),
      ];
    case "sessions.list":
      return [
        { keys: "J/K", label: "Session" },
        { keys: "GG/G", label: "First/last" },
        { keys: "L/Enter", label: "Open prompt" },
        { keys: "H", label: "Settings" },
        { keys: "P", label: "Pin reorder" },
      ];
    case "prompt.plan":
      return [
        { keys: "J/K", label: "Step" },
        { keys: "GG/G", label: "First/last" },
        { keys: "Enter", label: "Toggle" },
      ];
    case "prompt.queued":
    case "prompt.draft":
      return [
        { keys: "J/K", label: "Message" },
        { keys: "GG/G", label: "First/last" },
        { keys: "G→1…0", label: "Direct jump" },
        { keys: "L/Enter", label: "Edit" },
        { keys: "P", label: "Pin reorder" },
      ];
    case "conversation.transcript":
      return projection === "explore" ? EXPLORE_HINTS : HISTORY_HINTS;
    case "prompt.composer":
      return [
        { keys: "Esc", label: "Normal" },
        { keys: "Cmd+Enter", label: status === "busy" ? "Queue" : "Send" },
      ];
    default:
      return [];
  }
}

export function DesktopStatusLine({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const commands = useDesktopCommands();
  const { focusedPane, focusedRegion, mode } = workspace;
  const vimEnabled = useVimSetting();
  const vimMode = useVimMode();
  const ime = useImeStatus();
  const macro = useVimMacroRecording();
  const projection = useExploreSessionState(sessionId).projection;
  const connected = useStoreSelector((snapshot) => snapshot.connected);
  const promptEditorContext = focusedRegion?.startsWith("prompt.") === true;
  const resizingLayout = workspace.selectedSplitter !== null;
  const effectiveMode = resizingLayout
    ? "resize"
    : promptEditorContext && vimEnabled ? vimMode : mode;
  const modeColor = resizingLayout
    ? "primary.main"
    : promptEditorContext && vimEnabled
    ? (VIM_MODE_COLOR[vimMode] ?? "primary.main")
    : "primary.main";
  const imeLabel = ime.phase === "composing"
    ? (ime.autoInserted ? "IME → INSERT" : "IME · COMPOSING")
    : ime.phase === "committed"
    ? "IME · COMMITTED"
    : null;
  const regionElement = focusedRegion
    ? document.querySelector<HTMLElement>(
      `[data-desktop-region="${CSS.escape(focusedRegion)}"]`,
    )
    : null;
  const itemCount = regionElement && regionElement.dataset.desktopNavigation !== "scroll"
    ? [...regionElement.querySelectorAll<HTMLElement>("[data-desktop-item]")]
      .filter((element) => element.offsetParent !== null).length
    : 0;
  const promptRegions = focusedPane === "prompt" &&
      focusedRegion?.startsWith("prompt.") === true &&
      focusedRegion !== "prompt.composer"
    ? [
      { keys: DESKTOP_FOCUS_PLAN_SHORTCUT, label: "Plan" },
      { keys: "Mod+Y", label: "Queue" },
      { keys: "Mod+D", label: "Drafts" },
      { keys: DESKTOP_FOCUS_PROMPT_SHORTCUT, label: "Editor" },
    ]
    : [];
  const ordinaryHints = [
    ...promptRegions,
    ...regionHints(focusedRegion, status, projection),
    ...(focusedRegion === "sessions.list" && itemCount > 0
      ? [{ keys: "Mod+1…0", label: "Switch" }]
      : []),
    ...(regionElement?.dataset.desktopReorderable === "true" &&
        focusedRegion !== "sessions.list"
      ? [{ keys: "Mod+J/K", label: "Reorder" }]
      : []),
    ...(focusedRegion === "conversation.transcript" &&
      workspace.productMode === "agent"
      ? [{ keys: "Z", label: "Reading" }]
      : []),
    ...(focusedRegion === "conversation.transcript" &&
        document.querySelector("[data-desktop-permission-action='approve']")
      ? [{ keys: "A", label: "Allow" }]
      : []),
    ...(focusedRegion === "conversation.transcript" &&
        document.querySelector("[data-desktop-permission-action='reject']")
      ? [{ keys: "R", label: "Reject" }]
      : []),
    { keys: "Ctrl+W R", label: "Resize" },
  ];
  const hints = resizingLayout
    ? [
      { keys: "H/L", label: "Resize" },
      { keys: "Shift+H/L", label: "Large step" },
      { keys: "Tab", label: "Next bar" },
      { keys: "Esc/Enter", label: "Done" },
    ]
    : ordinaryHints;
  const paneLabel = resizingLayout
    ? "layout"
    : focusedRegion === "topbar.controls" ? "topbar" : focusedPane;
  const regionLabel = workspace.selectedSplitter ?? focusedRegion;

  return (
    <Box
      component="footer"
      data-desktop-status-line
      aria-label="Desktop status line"
      sx={{
        order: 2,
        position: "relative",
        zIndex: 2,
        minHeight: 29,
        display: "flex",
        alignItems: "center",
        borderTop: 1,
        borderColor: "divider",
        bgcolor: (theme) => alpha(theme.palette.background.paper, 0.56),
        color: "text.secondary",
        userSelect: "none",
        overflow: "hidden",
      }}
    >
      <Stack direction="row" alignItems="center" divider={<Divider orientation="vertical" flexItem />}>
        <Segment
          label={effectiveMode.toUpperCase()}
          color={modeColor}
          tooltip={promptEditorContext && vimEnabled
            ? `Editor Vim mode: ${vimMode}`
            : `Workspace mode: ${mode}`}
          mono
        />
        <Segment label={paneLabel.toUpperCase()} tooltip="Focused workspace pane" mono />
        {regionLabel && (
          <Segment
            label={(regionLabel.split(".").at(-1) ?? regionLabel).toUpperCase()}
            tooltip={resizingLayout ? "Selected resize bar" : "Focused region"}
            mono
          />
        )}
        {imeLabel && (
          <Segment
            label={imeLabel}
            color={ime.phase === "committed" ? "success.main" : "info.main"}
            tooltip={ime.autoInserted
              ? "Cowboy detected native composition and safely entered Vim Insert mode"
              : ime.phase === "committed"
              ? "Native IME composition committed"
              : "Native IME composition is active"}
            mono
          />
        )}
        {macro && (
          <Segment
            label={
              <Stack direction="row" spacing={0.6} alignItems="center">
                <Box
                  component="span"
                  sx={{
                    width: 6,
                    height: 6,
                    borderRadius: "50%",
                    bgcolor: "error.main",
                    boxShadow: (theme) =>
                      `0 0 0 3px ${alpha(theme.palette.error.main, 0.12)}`,
                  }}
                />
                <Box component="span">REC @{macro.register}</Box>
                <DesktopKeycap keyLabel="Q" quiet />
                <Box component="span" sx={{ color: "text.secondary" }}>Stop</Box>
              </Stack>
            }
            color="error.main"
            tooltip={`Recording Vim macro into register ${macro.register}. Press Q or click to stop.`}
            mono
            onClick={macro.stop}
          />
        )}
      </Stack>
      {hints.length > 0 && (
        <Stack
          direction="row"
          spacing={1.25}
          alignItems="center"
          aria-label="Focused region shortcuts"
          sx={{
            ml: 0.75,
            mr: 1,
            minWidth: 0,
            color: "text.disabled",
            "@media (max-width: 1180px)": { display: "none" },
          }}
        >
          {hints.map((hint) => (
            <Stack
              key={`${hint.keys}-${hint.label}`}
              direction="row"
              spacing={0.5}
              alignItems="center"
            >
              <DesktopShortcut shortcut={hint.keys} quiet />
              <Box component="span" sx={{ fontSize: "0.625rem", whiteSpace: "nowrap" }}>
                {hint.label}
              </Box>
            </Stack>
          ))}
        </Stack>
      )}
      <Box sx={{ flex: 1 }} />
      <Stack direction="row" alignItems="center" divider={<Divider orientation="vertical" flexItem />}>
        <Segment label={status.toUpperCase()} tooltip="Session status" mono />
        <Segment
          label={connected ? "CONNECTED" : "OFFLINE"}
          color={connected ? "success.main" : "error.main"}
          tooltip="Cowboy WebSocket connection"
          mono
        />
        <Segment
          label={
            <Stack direction="row" spacing={0.55} alignItems="center">
              <DesktopShortcut shortcut="Mod+K" quiet />
              <Box component="span">Commands</Box>
            </Stack>
          }
          tooltip="Open the Desktop command palette"
          mono
          onClick={(): void => {
            commands.execute("commandPalette.open");
          }}
        />
        <Segment
          label={
            <Stack direction="row" spacing={0.55} alignItems="center">
              <DesktopShortcut shortcut="Mod+/" quiet />
              <Box component="span">Shortcuts</Box>
            </Stack>
          }
          tooltip="Open the Desktop keyboard shortcut guide (Mod+/)"
          onClick={(): void => {
            commands.execute("shortcuts.open");
          }}
        />
      </Stack>
    </Box>
  );
}
