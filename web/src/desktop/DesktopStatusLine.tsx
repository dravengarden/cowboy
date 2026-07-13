import { alpha, Box, ButtonBase, Divider, Stack, Tooltip } from "@mui/material";
import type { Status } from "../protocol";
import { useStoreSelector } from "../store";
import { useVimMode, VIM_MODE_COLOR } from "../vimModeStore";
import { useVimSetting } from "../vimSetting";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopKeycap } from "./commands/DesktopKeycap";
import { useDesktopCommands } from "./commands/DesktopCommandProvider";
import { useImeStatus } from "./vim/imeStatusStore";

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

function regionHints(region: string | null, status: Status): RegionHint[] {
  switch (region) {
    case "topbar.controls":
      return [
        { keys: "H/L", label: "Select" },
        { keys: "Enter", label: "Open" },
        { keys: "R", label: "Config" },
        { keys: "U", label: "Usage" },
        { keys: "C", label: "Compact" },
        { keys: "F", label: "Follow" },
        ...(status === "busy" ? [{ keys: "S", label: "Stop" }] : []),
      ];
    case "sessions.list":
      return [
        { keys: "J/K", label: "Session" },
        { keys: "GG/G", label: "First/last" },
        { keys: "Enter", label: "Open" },
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
        { keys: "Enter", label: "Run" },
        { keys: "I", label: "Edit" },
      ];
    case "conversation.transcript":
      return [
        { keys: "J/K", label: "Event" },
        { keys: "GG/G", label: "First/last" },
        { keys: "Enter", label: "Toggle" },
      ];
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
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const commands = useDesktopCommands();
  const { focusedPane, focusedRegion, leaderPrefix, mode } = workspace;
  const vimEnabled = useVimSetting();
  const vimMode = useVimMode();
  const ime = useImeStatus();
  const connected = useStoreSelector((snapshot) => snapshot.connected);
  const effectiveMode = focusedPane === "prompt" && vimEnabled ? vimMode : mode;
  const modeColor = focusedPane === "prompt" && vimEnabled
    ? (VIM_MODE_COLOR[vimMode] ?? "primary.main")
    : "primary.main";
  const imeLabel = ime.phase === "composing"
    ? (ime.autoInserted ? "IME → INSERT" : "IME · COMPOSING")
    : ime.phase === "committed"
    ? "IME · COMMITTED"
    : null;
  const hints = regionHints(focusedRegion, status);

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
          tooltip={focusedPane === "prompt" && vimEnabled
            ? `Editor Vim mode: ${vimMode}`
            : `Workspace mode: ${mode}`}
          mono
        />
        <Segment label={focusedPane.toUpperCase()} tooltip="Focused workspace pane" mono />
        {focusedRegion && (
          <Segment
            label={(focusedRegion.split(".").at(-1) ?? focusedRegion).toUpperCase()}
            tooltip="Focused region"
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
              <DesktopKeycap keyLabel={hint.keys} quiet />
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
              <DesktopKeycap keyLabel="SPC" accent={mode === "leader"} />
              {leaderPrefix.map((key, index) => (
                <DesktopKeycap key={`${key}-${String(index)}`} keyLabel={key} accent />
              ))}
              {mode !== "leader" && <Box component="span">Commands</Box>}
            </Stack>
          }
          tooltip="Open the discoverable Desktop command board"
          mono
          onClick={(): void => {
            workspace.setLeaderPrefix([]);
            workspace.setLeaderMessage(null);
            workspace.setMode("leader");
          }}
        />
        <Segment
          label={
            <Stack direction="row" spacing={0.55} alignItems="center">
              <DesktopKeycap keyLabel="?" />
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
