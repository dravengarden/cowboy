import { useEffect, useMemo, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  InputAdornment,
  List,
  ListItemButton,
  ListItemText,
  TextField,
  Typography,
} from "@mui/material";
import { Search } from "@mui/icons-material";
import type { SessionMeta } from "../../protocol";
import {
  type DesktopCommand,
  useDesktopCommand,
  useDesktopCommands,
} from "./DesktopCommandProvider";

export function DesktopCommandHost({
  sessions,
  activeId,
  onPickSession,
  onNewSession,
  onFocusComposer,
  onTogglePromptPane,
  onOpenSettings,
}: {
  sessions: SessionMeta[];
  activeId: string | null;
  onPickSession: (id: string) => void;
  onNewSession: () => void;
  onFocusComposer: () => void;
  onTogglePromptPane: () => void;
  onOpenSettings: () => void;
}): React.JSX.Element {
  const registry = useDesktopCommands();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);

  const moveSession = (delta: number): void => {
    if (sessions.length === 0) return;
    const current = Math.max(
      0,
      sessions.findIndex((session) => session.id === activeId),
    );
    const next = (current + delta + sessions.length) % sessions.length;
    const session = sessions[next];
    if (session) onPickSession(session.id);
  };

  const commands = useMemo<DesktopCommand[]>(() => [
    {
      id: "commandPalette.open",
      title: "Open Command Palette",
      shortcut: "Mod+Shift+P",
      allowInEditor: true,
      run: () => {
        setQuery("");
        setSelected(0);
        setPaletteOpen(true);
      },
    },
    {
      id: "session.new",
      title: "New Session",
      shortcut: "Mod+N",
      allowInEditor: true,
      run: onNewSession,
    },
    {
      id: "session.previous",
      title: "Previous Session",
      shortcut: "Mod+Shift+[",
      allowInEditor: true,
      when: () => sessions.length > 1,
      run: () => moveSession(-1),
    },
    {
      id: "session.next",
      title: "Next Session",
      shortcut: "Mod+Shift+]",
      allowInEditor: true,
      when: () => sessions.length > 1,
      run: () => moveSession(1),
    },
    {
      id: "composer.focus",
      title: "Focus Composer",
      shortcut: "Mod+L",
      allowInEditor: true,
      when: () => activeId !== null,
      run: onFocusComposer,
    },
    {
      id: "workspace.togglePromptPane",
      title: "Toggle Prompt Pane",
      shortcut: "Mod+Alt+P",
      allowInEditor: true,
      run: onTogglePromptPane,
    },
    {
      id: "settings.open",
      title: "Open Settings",
      shortcut: "Mod+\u002c",
      allowInEditor: true,
      run: onOpenSettings,
    },
  ], [
    activeId,
    onFocusComposer,
    onNewSession,
    onOpenSettings,
    onPickSession,
    onTogglePromptPane,
    sessions,
  ]);

  // Hooks must be unconditional; the command list has a stable length and IDs.
  useDesktopCommand(commands[0] as DesktopCommand);
  useDesktopCommand(commands[1] as DesktopCommand);
  useDesktopCommand(commands[2] as DesktopCommand);
  useDesktopCommand(commands[3] as DesktopCommand);
  useDesktopCommand(commands[4] as DesktopCommand);
  useDesktopCommand(commands[5] as DesktopCommand);
  useDesktopCommand(commands[6] as DesktopCommand);

  const normalized = query.trim().toLowerCase();
  const available = registry.list().filter((command) =>
    command.id !== "commandPalette.open" &&
    command.when?.() !== false &&
    (!normalized ||
      `${command.title} ${command.id}`.toLowerCase().includes(normalized))
  );
  useEffect(() => setSelected(0), [query]);
  useEffect(() => {
    if (selected >= available.length) {
      setSelected(Math.max(0, available.length - 1));
    }
  }, [available.length, selected]);

  const runSelected = (): void => {
    const command = available[selected];
    if (!command) return;
    registry.execute(command.id);
    setPaletteOpen(false);
  };

  return (
    <Dialog
      open={paletteOpen}
      onClose={(): void => setPaletteOpen(false)}
      fullWidth
      maxWidth="sm"
      slotProps={{ paper: { sx: { alignSelf: "flex-start", mt: "12vh" } } }}
    >
      <DialogTitle sx={{ pb: 1 }}>Command Palette</DialogTitle>
      <DialogContent sx={{ px: 1.5, pb: 1.5 }}>
        <TextField
          autoFocus
          fullWidth
          size="small"
          value={query}
          onChange={(event): void => setQuery(event.target.value)}
          onKeyDown={(event): void => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setSelected((value) =>
                Math.min(value + 1, Math.max(0, available.length - 1))
              );
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setSelected((value) => Math.max(0, value - 1));
            } else if (event.key === "Enter") {
              event.preventDefault();
              runSelected();
            }
          }}
          placeholder="Search commands…"
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <Search fontSize="small" />
                </InputAdornment>
              ),
            },
          }}
        />
        <List
          dense
          sx={{ maxHeight: "min(52vh, 460px)", overflowY: "auto", pt: 1 }}
        >
          {available.map((command, index) => (
            <ListItemButton
              key={command.id}
              selected={index === selected}
              onMouseMove={(): void => setSelected(index)}
              onClick={(): void => {
                registry.execute(command.id);
                setPaletteOpen(false);
              }}
              sx={{ borderRadius: 1.5 }}
            >
              <ListItemText primary={command.title} secondary={command.id} />
              {command.shortcut && (
                <Typography
                  variant="caption"
                  color="text.secondary"
                  sx={{ ml: 2 }}
                >
                  {command.shortcut}
                </Typography>
              )}
            </ListItemButton>
          ))}
        </List>
      </DialogContent>
    </Dialog>
  );
}
