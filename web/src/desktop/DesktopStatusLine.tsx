import { alpha, Box, ButtonBase, Divider, Stack, Tooltip } from "@mui/material";
import type { Status } from "../protocol";
import { useStoreSelector } from "../store";
import { useVimMode, VIM_MODE_COLOR } from "../vimModeStore";
import { useVimSetting } from "../vimSetting";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopKeycap } from "./commands/DesktopKeycap";
import { useDesktopCommands } from "./commands/DesktopCommandProvider";

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
  const connected = useStoreSelector((snapshot) => snapshot.connected);
  const effectiveMode = focusedPane === "prompt" && vimEnabled ? vimMode : mode;
  const modeColor = focusedPane === "prompt" && vimEnabled
    ? (VIM_MODE_COLOR[vimMode] ?? "primary.main")
    : "primary.main";
  const imeAutoInsert = focusedPane === "prompt" && vimEnabled &&
    vimMode !== "insert" && vimMode !== "replace";

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
        {imeAutoInsert && (
          <Segment
            label="IME SAFE"
            color="info.main"
            tooltip="Normal commands use a non-editable focus target; CJK composition starts only in Insert"
            mono
          />
        )}
      </Stack>
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
