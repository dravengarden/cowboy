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
      elevation={0}
      sx={{
        position: "fixed",
        zIndex: (theme) => theme.zIndex.modal - 1,
        left: { xs: "50%", lg: "calc(50% + 144px)" },
        top: "50%",
        transform: "translate(-50%, -50%)",
        width: "min(680px, calc(100vw - 32px))",
        border: 1,
        borderColor: (theme) => alpha(theme.palette.primary.main, 0.24),
        borderRadius: 3,
        bgcolor: (theme) => alpha(
          theme.palette.background.paper,
          theme.palette.mode === "dark" ? 0.68 : 0.76,
        ),
        backgroundImage: (theme) =>
          `linear-gradient(145deg, ${alpha(theme.palette.common.white, theme.palette.mode === "dark" ? 0.06 : 0.42)}, transparent 55%)`,
        backdropFilter: "blur(28px) saturate(145%)",
        WebkitBackdropFilter: "blur(28px) saturate(145%)",
        boxShadow: (theme) => [
          `0 24px 70px ${alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.48 : 0.2)}`,
          `0 4px 18px ${alpha(theme.palette.primary.main, 0.12)}`,
          `inset 0 1px 0 ${alpha(theme.palette.common.white, theme.palette.mode === "dark" ? 0.08 : 0.62)}`,
        ].join(", "),
        overflow: "hidden",
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        spacing={1}
        sx={{
          px: 1.5,
          height: 42,
          borderBottom: 1,
          borderColor: (theme) => alpha(theme.palette.divider, 0.72),
        }}
      >
        <Chip
          label="SPC"
          size="small"
          color="primary"
          sx={{ height: 24, fontFamily: "monospace", fontWeight: 800 }}
        />
        {workspace.leaderPrefix.map((key, index) => (
          <Chip
            key={`${key}-${String(index)}`}
            label={key}
            size="small"
            sx={{ height: 24, fontFamily: "monospace", fontWeight: 700 }}
          />
        ))}
        <Typography
          variant="caption"
          color={workspace.leaderMessage ? "error.main" : "text.secondary"}
          sx={{ ml: 0.5 }}
        >
          {workspace.leaderMessage ?? "Choose a command · Esc closes · Backspace goes up"}
        </Typography>
      </Stack>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
          gap: 0.75,
          p: 1.25,
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
                minHeight: 54,
                px: 1.25,
                border: 1,
                borderColor: (theme) => alpha(theme.palette.divider, 0.58),
                borderRadius: 1.5,
                justifyContent: "flex-start",
                textAlign: "left",
                bgcolor: (theme) => alpha(theme.palette.background.paper, 0.28),
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
              <Box
                component="kbd"
                sx={{
                  width: 28,
                  height: 28,
                  mr: 1.25,
                  display: "grid",
                  placeItems: "center",
                  borderRadius: 1,
                  bgcolor: "primary.main",
                  color: "primary.contrastText",
                  fontFamily: "monospace",
                  fontWeight: 800,
                  boxShadow: (theme) => `0 3px 9px ${alpha(theme.palette.primary.main, 0.24)}`,
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
