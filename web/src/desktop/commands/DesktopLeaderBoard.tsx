import { alpha, Box, ButtonBase, Chip, Paper, Stack, Typography } from "@mui/material";
import { useMemo } from "react";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { useDesktopCommands, type DesktopCommand } from "./DesktopCommandProvider";

const GROUP_LABELS: Record<string, string> = {
  s: "Session",
  p: "Prompt",
  c: "Conversation",
  w: "Workspace",
  a: "Actions",
  g: "Go",
  f: "Find",
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
  const prefix = workspace.leaderPrefix.join("");
  const entries = useMemo<LeaderEntry[]>(() => {
    const byKey = new Map<string, LeaderEntry>();
    for (const command of registry.commands) {
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
  }, [prefix, registry.commands]);

  if (workspace.mode !== "leader") return null;

  const choose = (entry: LeaderEntry): void => {
    if (entry.command && !entry.branch) {
      if (entry.command.when?.() === false) {
        workspace.setLeaderMessage(typeof entry.command.disabledReason === "function"
          ? entry.command.disabledReason()
          : entry.command.disabledReason ?? "Command is unavailable in the current context");
        return;
      }
      registry.execute(entry.command.id);
      workspace.setLeaderPrefix([]);
      workspace.setLeaderMessage(null);
      workspace.setMode("normal");
      return;
    }
    workspace.setLeaderPrefix([...workspace.leaderPrefix, entry.key]);
    workspace.setLeaderMessage(null);
  };

  return (
    <Paper
      data-desktop-leader
      elevation={8}
      square
      sx={{
        position: "fixed",
        zIndex: (theme) => theme.zIndex.modal - 1,
        left: { xs: 16, lg: 304 },
        right: 16,
        bottom: 29,
        border: 1,
        borderColor: "divider",
        bgcolor: (theme) => alpha(theme.palette.background.paper, 0.97),
        backdropFilter: "blur(16px)",
        overflow: "hidden",
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        spacing={1}
        sx={{ px: 1.5, height: 36, borderBottom: 1, borderColor: "divider" }}
      >
        <Chip label="SPC" size="small" color="primary" sx={{ fontFamily: "monospace", fontWeight: 800 }} />
        {workspace.leaderPrefix.map((key, index) => (
          <Chip key={`${key}-${String(index)}`} label={key} size="small" sx={{ fontFamily: "monospace" }} />
        ))}
        <Typography variant="caption" color={workspace.leaderMessage ? "error.main" : "text.secondary"}>
          {workspace.leaderMessage ?? "Choose a command · Esc closes · Backspace goes up"}
        </Typography>
      </Stack>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(190px, 1fr))",
          gap: 0.5,
          p: 1,
          maxHeight: "min(38vh, 320px)",
          overflowY: "auto",
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
                minHeight: 48,
                px: 1,
                borderRadius: 1,
                justifyContent: "flex-start",
                textAlign: "left",
                "&:hover": { bgcolor: "action.hover" },
                opacity: disabled ? 0.48 : 1,
              }}
            >
              <Box
                component="kbd"
                sx={{
                  width: 26,
                  height: 26,
                  mr: 1,
                  display: "grid",
                  placeItems: "center",
                  borderRadius: 0.75,
                  bgcolor: "primary.main",
                  color: "primary.contrastText",
                  fontFamily: "monospace",
                  fontWeight: 800,
                }}
              >
                {entry.key}
              </Box>
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="body2" fontWeight={650} noWrap>{entry.label}</Typography>
                <Typography variant="caption" color="text.secondary" noWrap>
                  {reason ?? (entry.branch ? "More commands" : entry.command?.group)}
                </Typography>
              </Box>
            </ButtonBase>
          );
        })}
      </Box>
    </Paper>
  );
}
