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
} from "@mui/material";
import { Search } from "@mui/icons-material";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import {
  type DesktopCommand,
  useDesktopCommand,
  useDesktopCommands,
} from "./DesktopCommandProvider";
import { DesktopShortcut } from "./DesktopKeycap";
import { DesktopShortcutsDialog } from "./DesktopShortcutsDialog";

function DesktopCommandRegistration(
  { command }: { command: DesktopCommand },
): null {
  useDesktopCommand(command);
  return null;
}

export function DesktopCommandHost({
  onNewSession,
  onOpenSettings,
}: {
  onNewSession: () => void;
  onOpenSettings: () => void;
}): React.JSX.Element {
  const registry = useDesktopCommands();
  const workspace = useDesktopWorkspace();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
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
  const resolvePermission = (action: "approve" | "reject"): void => {
    document.querySelector<HTMLElement>(
      `[data-desktop-permission-action="${action}"]`,
    )?.click();
  };

  const commands = useMemo<DesktopCommand[]>(() => [
    {
      id: "shortcuts.open",
      title: "Keyboard Shortcuts",
      description: "Vim navigation and commands for the current Desktop context",
      group: "Help",
      shortcut: "Mod+/",
      allowInEditor: true,
      run: () => setShortcutsOpen(true),
    },
    {
      id: "commandPalette.open",
      title: "Open Command Palette",
      description: "Search every registered Desktop command",
      group: "Open",
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
      shortcut: "Mod+N",
      allowInEditor: true,
      run: onNewSession,
    },
    {
      id: "settings.open",
      title: "Open Settings",
      group: "Settings",
      shortcut: "Mod+\u002c",
      allowInEditor: true,
      run: onOpenSettings,
    },
    {
      id: "workspace.focusTopbar",
      title: "Focus Top Bar",
      description: "Move keyboard focus to session controls and usage",
      group: "Workspace",
      shortcut: "Mod+T",
      allowInEditor: true,
      when: () => document.querySelector("[data-desktop-region='topbar.controls']") !== null,
      run: () => workspace.focusRegion("topbar.controls"),
    },
    {
      id: "workspace.focusSessions",
      title: "Focus Sessions",
      group: "Workspace",
      shortcut: "Mod+E",
      allowInEditor: true,
      run: () => workspace.focusPane("sessions"),
    },
    {
      id: "workspace.focusPrompt",
      title: "Focus Prompt",
      group: "Workspace",
      shortcut: "Mod+P",
      allowInEditor: true,
      run: () => workspace.focusPane("prompt"),
    },
    {
      id: "workspace.focusConversation",
      title: "Focus Conversation",
      group: "Workspace",
      shortcut: "Mod+L",
      allowInEditor: true,
      run: () => workspace.focusPane("conversation"),
    },
    {
      id: "prompt.focusPlan",
      title: "Focus Plan",
      description: "Move keyboard focus to the current task plan",
      group: "Prompt",
      shortcut: "P",
      contexts: ["prompt"],
      // Preserve Vim paste once the editor has a document. On an empty Normal
      // canvas, the visible P hint may safely move focus to the task plan. The
      // command sink is the authoritative Normal-mode fact; React's mode and
      // preference stores can trail a native focus event by one render.
      allowInEditor: (target) => {
        const element = target instanceof Element ? target : null;
        const composer = element?.closest<HTMLElement>(
          "[data-desktop-region='prompt.composer'][data-desktop-editor-empty='true']",
        );
        return composer !== null &&
          (element?.matches("[data-vim-command-sink]") === true ||
            element?.closest(".cm-editor")?.classList.contains(
              "cm-vim-command-focused",
            ) === true);
      },
      when: () => document.querySelector("[data-desktop-region='prompt.plan']") !== null,
      disabledReason: "The agent has not published a plan",
      run: () => workspace.focusRegion("prompt.plan"),
    },
    {
      id: "prompt.focusQueue",
      title: "Open or Close Queue",
      description:
        "Open and focus queued prompts, or close them when the queue already owns focus",
      group: "Prompt",
      shortcut: "Mod+Y",
      allowInEditor: true,
      contexts: ["prompt"],
      when: () => document.querySelector("[data-desktop-region='prompt.queued']") !== null,
      disabledReason: "The queue is empty",
      run: () => {
        const toggle = document.querySelector<HTMLElement>(
          "[data-desktop-collapse-toggle='queued']",
        );
        if (!toggle) return;
        if (workspace.focusedRegion === "prompt.queued") {
          if (toggle.getAttribute("aria-expanded") === "true") toggle.click();
          requestAnimationFrame(() => workspace.focusRegion("prompt.composer"));
          return;
        }
        if (toggle.getAttribute("aria-expanded") === "false") toggle.click();
        requestAnimationFrame(() => workspace.focusRegion("prompt.queued"));
      },
    },
    {
      id: "prompt.focusDrafts",
      title: "Focus Drafts",
      description: "Move keyboard focus to parked drafts",
      group: "Prompt",
      shortcut: "Mod+D",
      allowInEditor: true,
      contexts: ["prompt"],
      when: () => document.querySelector("[data-desktop-region='prompt.draft']") !== null,
      disabledReason: "There are no drafts",
      run: () => {
        const toggle = document.querySelector<HTMLElement>(
          "[data-desktop-collapse-toggle='draft']",
        );
        if (toggle?.getAttribute("aria-expanded") === "false") toggle.click();
        requestAnimationFrame(() => workspace.focusRegion("prompt.draft"));
      },
    },
    {
      id: "conversation.focusTranscript",
      title: "Focus Transcript",
      group: "Conversation",
      shortcut: "C",
      contexts: ["conversation"],
      run: () => workspace.focusRegion("conversation.transcript"),
    },
    {
      id: "conversation.permissionApprove",
      title: "Allow Pending Permission",
      description: "Choose the least persistent available allow option",
      group: "Conversation",
      shortcut: "A",
      regions: ["conversation.transcript"],
      when: () =>
        document.querySelector("[data-desktop-permission-action='approve']") !== null,
      disabledReason: "No permission is awaiting approval",
      run: () => resolvePermission("approve"),
    },
    {
      id: "conversation.permissionReject",
      title: "Reject Pending Permission",
      description: "Choose the least persistent available reject option",
      group: "Conversation",
      shortcut: "R",
      regions: ["conversation.transcript"],
      when: () =>
        document.querySelector("[data-desktop-permission-action='reject']") !== null,
      disabledReason: "No permission is awaiting rejection",
      run: () => resolvePermission("reject"),
    },
    {
      id: "item.activate",
      title: "Activate Focused Item",
      description: "Run the primary action for the selected queue or draft row",
      group: "Actions",
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
      regions: ["prompt.queued", "prompt.draft"],
      when: () => document.activeElement?.closest("[data-desktop-item]") !== null,
      disabledReason: "Focus a queue or draft item first",
      run: () => clickFocusedItemAction("edit"),
    },
  ], [
    onNewSession,
    onOpenSettings,
    workspace,
  ]);

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
      {commands.map((command) => (
        <DesktopCommandRegistration key={command.id} command={command} />
      ))}
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
