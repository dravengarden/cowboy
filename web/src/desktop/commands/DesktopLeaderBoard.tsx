import { alpha, Box, ButtonBase, Modal, Paper, Stack, Typography } from "@mui/material";
import { useMemo, useRef } from "react";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { useDesktopCommands, type DesktopCommand } from "./DesktopCommandProvider";
import { DesktopKeycap } from "./DesktopKeycap";

const GROUP_LABELS: Record<string, string> = {
  s: "Session",
  p: "Prompt",
  c: "Conversation",
  w: "Workspace",
  a: "Actions",
  g: "Go",
  f: "Find",
  h: "Help",
  o: "Open",
  x: "Stop / cancel",
  ",": "Settings",
  "?": "All commands",
};

interface LeaderEntry {
  key: string;
  label: string;
  command?: DesktopCommand;
  branch: boolean;
}

export function DesktopLeaderBoard(): React.JSX.Element | null {
  const registry = useDesktopCommands();
  const workspace = useDesktopWorkspace();
  const returnFocusRef = useRef(
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );
  const prefix = workspace.leaderPrefix.join("");
  const entries = useMemo<LeaderEntry[]>(() => {
    const byKey = new Map<string, LeaderEntry>();
    for (const command of registry.commands) {
      if (command.contexts && !command.contexts.includes(workspace.focusedPane)) continue;
      if (
        command.regions &&
        (!workspace.focusedRegion || !command.regions.includes(workspace.focusedRegion))
      ) continue;
      const sequence = command.leader?.split(/\s+/).join("");
      if (!sequence?.startsWith(prefix) || sequence.length <= prefix.length) continue;
      const key = sequence[prefix.length] as string;
      const exact = sequence.length === prefix.length + 1;
      const current = byKey.get(key);
      byKey.set(key, {
        key,
        label: exact ? command.title : GROUP_LABELS[key] ?? command.group,
        ...(exact ? { command } : current?.command ? { command: current.command } : {}),
        branch: !exact || current?.branch === true,
      });
    }
    return [...byKey.values()].sort((left, right) => left.key.localeCompare(right.key));
  }, [prefix, registry.commands, workspace.focusedPane, workspace.focusedRegion]);

  if (workspace.mode !== "leader") return null;

  const choose = (entry: LeaderEntry): void => {
    if (entry.command && !entry.branch) {
      if (entry.command.when?.() === false) {
        workspace.setLeaderMessage(typeof entry.command.disabledReason === "function"
          ? entry.command.disabledReason()
          : entry.command.disabledReason ?? "Command is unavailable in the current context");
        return;
      }
      const commandId = entry.command.id;
      workspace.setLeaderPrefix([]);
      workspace.setLeaderMessage(null);
      workspace.setMode("normal");
      requestAnimationFrame(() => registry.execute(commandId));
      return;
    }
    workspace.setLeaderPrefix([...workspace.leaderPrefix, entry.key]);
    workspace.setLeaderMessage(null);
  };

  const close = (): void => {
    workspace.setLeaderPrefix([]);
    workspace.setLeaderMessage(null);
    workspace.setMode("normal");
    requestAnimationFrame(() => returnFocusRef.current?.focus({ preventScroll: true }));
  };

  return (
    <Modal
      open
      disableRestoreFocus
      onClose={close}
      aria-label="Desktop command board"
      slotProps={{
        backdrop: {
          sx: {
            // Preserve spatial context: the command board is a transient keyboard
            // aid, not a context switch. A light neutral scrim gives it elevation
            // without turning the entire three-pane workspace into an unreadable
            // blur. The board itself keeps its local frosted material.
            bgcolor: (theme) => alpha(
              theme.palette.common.black,
              theme.palette.mode === "dark" ? 0.28 : 0.1,
            ),
            backdropFilter: "none",
            WebkitBackdropFilter: "none",
          },
        },
      }}
    >
      <Paper
        data-desktop-leader
        elevation={0}
        sx={{
          position: "absolute",
          left: "50%",
          top: "50%",
          transform: "translate(-50%, -50%)",
          width: "min(620px, calc(100vw - 32px))",
          border: 1,
          borderColor: (theme) => alpha(theme.palette.primary.main, 0.26),
          borderRadius: 2.5,
          bgcolor: (theme) => alpha(theme.palette.background.paper, theme.palette.mode === "dark" ? 0.96 : 0.92),
          backgroundImage: (theme) =>
            `linear-gradient(145deg, ${alpha(theme.palette.common.white, theme.palette.mode === "dark" ? 0.05 : 0.46)}, transparent 55%)`,
          backdropFilter: "blur(30px) saturate(150%)",
          WebkitBackdropFilter: "blur(30px) saturate(150%)",
          boxShadow: (theme) => [
            `0 24px 70px ${alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.5 : 0.2)}`,
            `0 4px 18px ${alpha(theme.palette.primary.main, 0.12)}`,
            `inset 0 1px 0 ${alpha(theme.palette.common.white, theme.palette.mode === "dark" ? 0.07 : 0.68)}`,
          ].join(", "),
          outline: "none",
          overflow: "hidden",
        }}
      >
        <Stack
          direction="row"
          alignItems="center"
          spacing={0.65}
          sx={{
            px: 1.5,
            minHeight: 42,
            borderBottom: 1,
            borderColor: (theme) => alpha(theme.palette.divider, 0.72),
          }}
        >
          <DesktopKeycap keyLabel="SPC" accent />
          {workspace.leaderPrefix.map((key, index) => (
            <DesktopKeycap key={`${key}-${String(index)}`} keyLabel={key} accent />
          ))}
          <Typography
            variant="caption"
            color={workspace.leaderMessage ? "error.main" : "text.secondary"}
            noWrap
            sx={{ ml: 0.5, flex: 1 }}
          >
            {workspace.leaderMessage ?? "Choose a command"}
          </Typography>
          <DesktopKeycap keyLabel="Backspace" />
          <DesktopKeycap keyLabel="Esc" />
        </Stack>
        <Box
          sx={{
            display: "grid",
            gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
            gap: 0.65,
            p: 1.25,
            maxHeight: "min(42vh, 320px)",
            overflowY: "auto",
            "@media (max-width: 760px)": { gridTemplateColumns: "repeat(2, minmax(0, 1fr))" },
          }}
        >
          {entries.map((entry) => {
          const disabled = entry.command?.when?.() === false;
          const reason = disabled
            ? (typeof entry.command?.disabledReason === "function"
              ? entry.command.disabledReason()
              : entry.command?.disabledReason ?? "Unavailable")
            : entry.command?.description;
          return (
            <ButtonBase
              key={entry.key}
              aria-disabled={disabled || undefined}
              onClick={(): void => choose(entry)}
              sx={{
                minHeight: 52,
                px: 1,
                border: 1,
                borderColor: (theme) => alpha(theme.palette.divider, 0.58),
                borderRadius: 1.5,
                justifyContent: "flex-start",
                textAlign: "left",
                bgcolor: (theme) => alpha(theme.palette.background.default, 0.3),
                transition: "background-color 100ms, border-color 100ms, transform 100ms",
                "&:hover": {
                  bgcolor: (theme) => alpha(theme.palette.primary.main, 0.1),
                  borderColor: (theme) => alpha(theme.palette.primary.main, 0.34),
                  transform: "translateY(-1px)",
                },
                "&:focus-visible": {
                  outline: "2px solid",
                  outlineColor: "primary.main",
                  outlineOffset: -2,
                },
                opacity: disabled ? 0.48 : 1,
              }}
            >
              <Box sx={{ mr: 1 }}><DesktopKeycap keyLabel={entry.key} accent /></Box>
              <Box sx={{ minWidth: 0 }}>
                <Typography
                  variant="body2"
                  fontWeight={650}
                  noWrap
                  sx={{ display: "block", overflow: "hidden", textOverflow: "ellipsis" }}
                >
                  {entry.label}
                </Typography>
                <Typography
                  variant="caption"
                  color="text.secondary"
                  noWrap
                  sx={{ display: "block", overflow: "hidden", textOverflow: "ellipsis" }}
                >
                  {reason ?? (entry.branch ? "More commands" : entry.command?.group)}
                </Typography>
              </Box>
            </ButtonBase>
          );
          })}
        </Box>
      </Paper>
    </Modal>
  );
}
