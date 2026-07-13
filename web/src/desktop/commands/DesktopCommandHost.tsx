import { useEffect, useMemo, useRef, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  InputAdornment,
  List,
  ListItemButton,
  ListItemText,
  TextField,
} from "@mui/material";
import { Search } from "@mui/icons-material";
import type { SessionMeta } from "../../protocol";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { DesktopLeaderBoard } from "./DesktopLeaderBoard";
import {
  type DesktopCommand,
  useDesktopCommand,
  useDesktopCommands,
} from "./DesktopCommandProvider";
import { DesktopShortcut } from "./DesktopKeycap";
import { DesktopShortcutsDialog } from "./DesktopShortcutsDialog";
import { useVimMode } from "../../vimModeStore";
import { isMac } from "../../platform";
import { isTextEditingTarget } from "./shortcut";
import { isImeComposing } from "../vim/imeStatusStore";
import { desktopRegionShortcut } from "../DesktopRegionShortcut";

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
  const workspace = useDesktopWorkspace();
  const vimMode = useVimMode();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const numberedStateRef = useRef({
    sessions,
    onPickSession,
    vimMode,
    workspaceMode: workspace.mode,
  });
  numberedStateRef.current = {
    sessions,
    onPickSession,
    vimMode,
    workspaceMode: workspace.mode,
  };

  // Stable hot path for the ten visible session slots. Session status updates
  // replace the sessions array frequently; dynamic command registration can
  // briefly sit between effect cleanup and re-registration. A single lifetime
  // listener plus current refs makes Cmd+Digit atomic across those broadcasts.
  useEffect(() => {
    const onNumberedSession = (event: KeyboardEvent): void => {
      const mod = isMac
        ? event.metaKey && !event.ctrlKey
        : event.ctrlKey && !event.metaKey;
      if (
        !mod || event.shiftKey || event.altKey || event.repeat || event.isComposing ||
        isImeComposing() || event.keyCode === 229 ||
        isTextEditingTarget(event.target)
      ) return;
      const match = /^(?:Digit|Numpad)([0-9])$/.exec(event.code);
      if (!match?.[1]) return;
      const state = numberedStateRef.current;
      if (
        state.workspaceMode !== "normal" || state.vimMode !== "normal" ||
        document.querySelector("[role='dialog'], [role='menu']") !== null
      ) return;
      const digit = Number(match[1]);
      const index = digit === 0 ? 9 : digit - 1;
      const session = state.sessions[index];
      if (!session) return;
      event.preventDefault();
      event.stopPropagation();
      state.onPickSession(session.id);
    };
    globalThis.addEventListener("keydown", onNumberedSession, true);
    return () => globalThis.removeEventListener("keydown", onNumberedSession, true);
  }, []);

  // Session slots follow the visible sidebar order: ⌘1…⌘9, then ⌘0. Register
  // them dynamically so Command Palette and the shortcuts dialog discover the
  // same commands as the keycaps. They are intentionally unavailable in Vim
  // Insert/Replace, text controls, and any modal/menu interaction.
  useEffect(() => {
    const modalOpen = (): boolean =>
      document.querySelector("[role='dialog'], [role='menu']") !== null;
    const unregister = sessions.slice(0, 10).map((session, index) => {
      const digit = index === 9 ? "0" : String(index + 1);
      return registry.register({
        id: `session.switch.${digit}`,
        title: `Switch to Session ${digit}: ${session.title}`,
        description: "Select the numbered session shown in the Desktop sidebar",
        group: "Session",
        shortcut: `Mod+${digit}`,
        when: () => workspace.mode === "normal" && vimMode === "normal" && !modalOpen(),
        run: () => onPickSession(session.id),
      });
    });
    return () => unregister.forEach((remove) => remove());
  }, [onPickSession, registry.register, sessions, vimMode, workspace.mode]);

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

  const clickFocusedItemAction = (action: "default" | "edit"): void => {
    const item = document.activeElement instanceof HTMLElement
      ? document.activeElement.closest<HTMLElement>("[data-desktop-item]")
      : null;
    item?.querySelector<HTMLElement>(
      action === "default"
        ? "[data-desktop-item-action='default']"
        : "[data-desktop-item-action='edit'], button[aria-label='Edit']",
    )?.click();
  };

  const commands = useMemo<DesktopCommand[]>(() => [
    {
      id: "shortcuts.open",
      title: "Keyboard Shortcuts",
      description: "Vim navigation and commands for the current Desktop context",
      group: "Help",
      leader: "h k",
      shortcut: "Mod+/",
      allowInEditor: true,
      run: () => setShortcutsOpen(true),
    },
    {
      id: "commandPalette.open",
      title: "Open Command Palette",
      description: "Search every registered Desktop command",
      group: "Open",
      leader: "?",
      shortcut: "Mod+K",
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
      description: "Create a Cowboy session",
      group: "Session",
      leader: "s n",
      shortcut: "Mod+N",
      allowInEditor: true,
      run: onNewSession,
    },
    {
      id: "session.previous",
      title: "Previous Session",
      group: "Session",
      leader: "s k",
      shortcut: "Alt+K",
      allowInEditor: true,
      when: () => sessions.length > 1,
      disabledReason: "Only one session is available",
      run: () => moveSession(-1),
    },
    {
      id: "session.next",
      title: "Next Session",
      group: "Session",
      leader: "s j",
      shortcut: "Alt+J",
      allowInEditor: true,
      when: () => sessions.length > 1,
      disabledReason: "Only one session is available",
      run: () => moveSession(1),
    },
    {
      id: "composer.focus",
      title: "Focus Composer",
      group: "Prompt",
      leader: "p f",
      shortcut: "Mod+L",
      allowInEditor: true,
      when: () => activeId !== null,
      disabledReason: "No active session",
      run: onFocusComposer,
    },
    {
      id: "workspace.togglePromptPane",
      title: "Toggle Prompt Pane",
      group: "Workspace",
      leader: "w p",
      shortcut: "Alt+P",
      allowInEditor: true,
      run: onTogglePromptPane,
    },
    {
      id: "settings.open",
      title: "Open Settings",
      group: "Settings",
      leader: ",",
      shortcut: "Mod+\u002c",
      allowInEditor: true,
      run: onOpenSettings,
    },
    {
      id: "workspace.focusTopbar",
      title: "Focus Top Bar",
      description: "Move keyboard focus to session controls and usage",
      group: "Workspace",
      leader: "w t",
      shortcut: desktopRegionShortcut("T"),
      allowInEditor: true,
      when: () => document.querySelector("[data-desktop-region='topbar.controls']") !== null,
      run: () => workspace.focusRegion("topbar.controls"),
    },
    {
      id: "workspace.focusSessions",
      title: "Focus Sessions",
      group: "Workspace",
      leader: "w s",
      shortcut: desktopRegionShortcut("S"),
      allowInEditor: true,
      run: () => workspace.focusPane("sessions"),
    },
    {
      id: "workspace.focusPrompt",
      title: "Focus Prompt",
      group: "Workspace",
      leader: "w e",
      run: () => workspace.focusPane("prompt"),
    },
    {
      id: "workspace.focusConversation",
      title: "Focus Conversation",
      group: "Workspace",
      leader: "w c",
      run: () => workspace.focusPane("conversation"),
    },
    {
      id: "prompt.focusPlan",
      title: "Focus Plan",
      description: "Move keyboard focus to the current task plan",
      group: "Prompt",
      leader: "p l",
      shortcut: desktopRegionShortcut("P"),
      allowInEditor: true,
      when: () => document.querySelector("[data-desktop-region='prompt.plan']") !== null,
      disabledReason: "The agent has not published a plan",
      run: () => workspace.focusRegion("prompt.plan"),
    },
    {
      id: "prompt.focusQueue",
      title: "Focus Queue",
      description: "Move keyboard focus to queued prompts",
      group: "Prompt",
      leader: "p q",
      shortcut: desktopRegionShortcut("Q"),
      allowInEditor: true,
      when: () => document.querySelector("[data-desktop-region='prompt.queued']") !== null,
      disabledReason: "The queue is empty",
      run: () => workspace.focusRegion("prompt.queued"),
    },
    {
      id: "prompt.focusDrafts",
      title: "Focus Drafts",
      description: "Move keyboard focus to parked drafts",
      group: "Prompt",
      leader: "p d",
      shortcut: desktopRegionShortcut("D"),
      allowInEditor: true,
      when: () => document.querySelector("[data-desktop-region='prompt.draft']") !== null,
      disabledReason: "There are no drafts",
      run: () => workspace.focusRegion("prompt.draft"),
    },
    {
      id: "prompt.focusEditor",
      title: "Focus Prompt Editor",
      group: "Prompt",
      leader: "p e",
      shortcut: desktopRegionShortcut("E"),
      allowInEditor: true,
      run: () => workspace.focusRegion("prompt.composer"),
    },
    {
      id: "conversation.focusTranscript",
      title: "Focus Transcript",
      group: "Conversation",
      leader: "c c",
      shortcut: desktopRegionShortcut("C"),
      allowInEditor: true,
      run: () => workspace.focusRegion("conversation.transcript"),
    },
    {
      id: "item.activate",
      title: "Activate Focused Item",
      description: "Run the primary action for the selected queue or draft row",
      group: "Actions",
      leader: "a",
      regions: ["prompt.queued", "prompt.draft"],
      when: () => document.activeElement?.closest("[data-desktop-item]") !== null,
      disabledReason: "Focus a queue or draft item first",
      run: () => clickFocusedItemAction("default"),
    },
    {
      id: "item.edit",
      title: "Edit Focused Item",
      description: "Open the selected queue or draft row in the editor",
      group: "Actions",
      leader: "e",
      regions: ["prompt.queued", "prompt.draft"],
      when: () => document.activeElement?.closest("[data-desktop-item]") !== null,
      disabledReason: "Focus a queue or draft item first",
      run: () => clickFocusedItemAction("edit"),
    },
  ], [
    activeId,
    onFocusComposer,
    onNewSession,
    onOpenSettings,
    onPickSession,
    onTogglePromptPane,
    sessions,
    workspace,
  ]);

  // Hooks must be unconditional; the command list has a stable length and IDs.
  useDesktopCommand(commands[0] as DesktopCommand);
  useDesktopCommand(commands[1] as DesktopCommand);
  useDesktopCommand(commands[2] as DesktopCommand);
  useDesktopCommand(commands[3] as DesktopCommand);
  useDesktopCommand(commands[4] as DesktopCommand);
  useDesktopCommand(commands[5] as DesktopCommand);
  useDesktopCommand(commands[6] as DesktopCommand);
  useDesktopCommand(commands[7] as DesktopCommand);
  useDesktopCommand(commands[8] as DesktopCommand);
  useDesktopCommand(commands[9] as DesktopCommand);
  useDesktopCommand(commands[10] as DesktopCommand);
  useDesktopCommand(commands[11] as DesktopCommand);
  useDesktopCommand(commands[12] as DesktopCommand);
  useDesktopCommand(commands[13] as DesktopCommand);
  useDesktopCommand(commands[14] as DesktopCommand);
  useDesktopCommand(commands[15] as DesktopCommand);
  useDesktopCommand(commands[16] as DesktopCommand);
  useDesktopCommand(commands[17] as DesktopCommand);
  useDesktopCommand(commands[18] as DesktopCommand);

  const normalized = query.trim().toLowerCase();
  const available = registry.list().filter((command) =>
    command.id !== "commandPalette.open" &&
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
    if (registry.execute(command.id)) setPaletteOpen(false);
  };

  return (
    <>
      <DesktopLeaderBoard />
      <DesktopShortcutsDialog
        open={shortcutsOpen}
        onClose={(): void => setShortcutsOpen(false)}
      />
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
              disabled={command.when?.() === false}
              onMouseMove={(): void => setSelected(index)}
              onClick={(): void => {
                if (registry.execute(command.id)) setPaletteOpen(false);
              }}
              sx={{ borderRadius: 1.5 }}
            >
              <ListItemText
                primary={command.title}
                secondary={command.when?.() === false
                  ? (typeof command.disabledReason === "function"
                    ? command.disabledReason()
                    : command.disabledReason ?? "Unavailable")
                  : command.description ?? command.id}
              />
              {command.shortcut && (
                <DesktopShortcut shortcut={command.shortcut} />
              )}
            </ListItemButton>
          ))}
        </List>
      </DialogContent>
      </Dialog>
    </>
  );
}
