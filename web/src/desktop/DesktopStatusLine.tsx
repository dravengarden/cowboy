import { alpha, Box, ButtonBase, Divider, Stack, Tooltip } from "@mui/material";
import { useMemo } from "react";
import type { Status } from "../protocol";
import { useStoreSelector } from "../store";
import { useVimMode, VIM_MODE_COLOR } from "../vimModeStore";
import { useVimSetting } from "../vimSetting";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";

const EMPTY_CONFIG_OPTIONS: never[] = [];

function Segment({
  label,
  tooltip,
  color = "text.secondary",
  mono = false,
  onClick,
}: {
  label: string;
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
        fontFamily: mono ? "monospace" : "inherit",
        "&:hover": { bgcolor: "action.hover", color: "text.primary" },
      }}
    >
      {label}
    </ButtonBase>
  );
  return tooltip ? <Tooltip title={tooltip}>{body}</Tooltip> : body;
}

export function DesktopStatusLine({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const { focusedPane, leaderPrefix, mode } = workspace;
  const vimEnabled = useVimSetting();
  const vimMode = useVimMode();
  const connected = useStoreSelector((snapshot) => snapshot.connected);
  const session = useStoreSelector((snapshot) =>
    snapshot.sessions.find((candidate) => candidate.id === sessionId)
  );
  const configOptionsBySession = useStoreSelector((snapshot) => snapshot.configOptions);
  const configOptions = configOptionsBySession.get(sessionId) ?? EMPTY_CONFIG_OPTIONS;
  const config = useMemo(
    () => new Map(configOptions.map((option) => [option.id, String(option.currentValue)])),
    [configOptions],
  );
  const effectiveMode = focusedPane === "prompt" && vimEnabled ? vimMode : mode;
  const modeColor = focusedPane === "prompt" && vimEnabled
    ? (VIM_MODE_COLOR[vimMode] ?? "primary.main")
    : "primary.main";
  const imeAutoInsert = focusedPane === "prompt" && vimEnabled &&
    vimMode !== "insert" && vimMode !== "replace";
  const contextPercent = session?.context_size
    ? Math.round(((session.context_used ?? 0) / session.context_size) * 100)
    : null;

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
        {imeAutoInsert && (
          <Segment
            label="IME → INSERT AUTO"
            color="info.main"
            tooltip="Starting Chinese/Japanese/Korean composition automatically enters Insert"
            mono
          />
        )}
        <Segment
          label={session?.provider ?? "NO PROVIDER"}
          tooltip={session?.title ?? "No active session"}
        />
        {config.get("model") && <Segment label={config.get("model") as string} tooltip="Model" />}
        {config.get("effort") && <Segment label={config.get("effort") as string} tooltip="Reasoning effort" />}
        {contextPercent !== null && <Segment label={`${String(contextPercent)}% ctx`} tooltip="Context window used" />}
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
          label={mode === "leader" ? `SPC ${leaderPrefix.join(" ")}`.trim() : "SPC commands"}
          tooltip="Open the discoverable Desktop command board"
          mono
          onClick={(): void => {
            workspace.setLeaderPrefix([]);
            workspace.setLeaderMessage(null);
            workspace.setMode("leader");
          }}
        />
        <Segment label="f hints · soon" tooltip="Vimium target hints arrive in Phase 4" mono />
      </Stack>
    </Box>
  );
}
