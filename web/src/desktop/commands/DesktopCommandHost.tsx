import { useEffect, useMemo, useRef, useState } from "react";
import {
  alpha,
  Box,
  InputBase,
  List,
  ListItemButton,
  ListItemText,
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
import {
  DESKTOP_FOCUS_PLAN_SHORTCUT,
  DESKTOP_FOCUS_PROMPT_SHORTCUT,
  DESKTOP_RESIZE_NARROW_SHORTCUT,
  DESKTOP_RESIZE_SELECT_SHORTCUT,
  DESKTOP_RESIZE_WIDEN_SHORTCUT,
} from "./workspaceShortcuts";
import {
  DESKTOP_SPLITTER_ADJUST_EVENT,
  DESKTOP_SPLITTER_STEP,
  preferredDesktopSplitter,
  resolveDesktopResizeSplitter,
  visibleDesktopSplitterIds,
} from "../desktopSplitterKeyboard";
import { DESKTOP_INSET_RADIUS } from "../DesktopEmbeddedControl";
import { DesktopModal } from "../DesktopModal";
import { desktopImeOwnsKey } from "./imeShortcut";

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
  const paletteInputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!paletteOpen) return undefined;
    const frame = requestAnimationFrame(() => paletteInputRef.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [paletteOpen]);
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
      title: "Focus Message the Agent",
      description: "Return to the Prompt editor in Vim Normal mode",
      group: "Workspace",
      shortcut: DESKTOP_FOCUS_PROMPT_SHORTCUT,
      allowInEditor: true,
      run: () => workspace.focusRegion("prompt.composer"),
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
      id: "workspace.enterResize",
      title: "Select Layout Resize Bar",
      description:
        "Enter Resize mode on the nearest vertical split, then H/L to move it",
      group: "Workspace",
      shortcut: DESKTOP_RESIZE_SELECT_SHORTCUT,
      allowInEditor: true,
      run: () => {
        if (workspace.selectedSplitter !== null) {
          workspace.setSelectedSplitter(null);
          if (workspace.focusedRegion) {
            requestAnimationFrame(() =>
              workspace.focusRegion(workspace.focusedRegion as string)
            );
          }
          return;
        }
        const splitter = preferredDesktopSplitter(
          visibleDesktopSplitterIds(),
          workspace.focusedPane,
          workspace.productMode,
        );
        if (splitter) workspace.setSelectedSplitter(splitter);
      },
    },
    {
      id: "workspace.resizeNarrow",
      title: "Narrow Nearest Pane",
      description: "Shrink the nearest visible vertical split and enter Resize mode",
      group: "Workspace",
      shortcut: DESKTOP_RESIZE_NARROW_SHORTCUT,
      allowInEditor: true,
      run: () => {
        const splitter = resolveDesktopResizeSplitter(
          visibleDesktopSplitterIds(),
          workspace.selectedSplitter,
          workspace.focusedPane,
          workspace.productMode,
        );
        if (!splitter) return;
        workspace.setSelectedSplitter(splitter);
        globalThis.dispatchEvent(new CustomEvent(DESKTOP_SPLITTER_ADJUST_EVENT, {
          detail: { splitter, delta: -DESKTOP_SPLITTER_STEP },
        }));
      },
    },
    {
      id: "workspace.resizeWiden",
      title: "Widen Nearest Pane",
      description: "Grow the nearest visible vertical split and enter Resize mode",
      group: "Workspace",
      shortcut: DESKTOP_RESIZE_WIDEN_SHORTCUT,
      allowInEditor: true,
      run: () => {
        const splitter = resolveDesktopResizeSplitter(
          visibleDesktopSplitterIds(),
          workspace.selectedSplitter,
          workspace.focusedPane,
          workspace.productMode,
        );
        if (!splitter) return;
        workspace.setSelectedSplitter(splitter);
        globalThis.dispatchEvent(new CustomEvent(DESKTOP_SPLITTER_ADJUST_EVENT, {
          detail: { splitter, delta: DESKTOP_SPLITTER_STEP },
        }));
      },
    },
    {
      id: "prompt.focusPlan",
      title: "Focus Plan",
      description: "Move keyboard focus to the current task plan",
      group: "Prompt",
      shortcut: DESKTOP_FOCUS_PLAN_SHORTCUT,
      allowInEditor: true,
      when: () => document.querySelector("[data-desktop-region='prompt.plan']") !== null,
      disabledReason: "The agent has not published a plan",
      // Mod+P is Cowboy's global Plan command. Consume it while Plan is absent
      // so the same chord never falls through to the browser's Print action.
      consumeWhenDisabled: true,
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
      id: "conversation.enterReadingMode",
      title: "Enter Reading Mode",
      description: "Open the conversation in a distraction-free reading workspace",
      group: "Conversation",
      shortcut: "Z",
      contexts: ["conversation"],
      run: () => {
        workspace.setProductMode("reading");
        requestAnimationFrame(() => workspace.focusRegion("conversation.transcript"));
      },
    },
    {
      id: "conversation.toggleFollow",
      title: "Toggle Following",
      description: "Jump to the latest output or pause automatic following",
      group: "Conversation",
      shortcut: "F",
      contexts: ["conversation"],
      run: () => {
        document.querySelector<HTMLButtonElement>(
          "[data-desktop-conversation-follow]",
        )?.click();
      },
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
  const selectedCommand = available[selected];
  const selectedAvailable = selectedCommand !== undefined &&
    selectedCommand.when?.() !== false;

  return (
    <>
      {commands.map((command) => (
        <DesktopCommandRegistration key={command.id} command={command} />
      ))}
      <DesktopShortcutsDialog
        open={shortcutsOpen}
        onClose={(): void => setShortcutsOpen(false)}
      />
      <DesktopModal
        open={paletteOpen}
        onClose={(): void => setPaletteOpen(false)}
        title="Command Palette"
        description="Search and run every registered Desktop command."
        width={680}
        shortcutGroups={[
          {
            slots: [
              { shortcut: "Mod+K", label: "Palette", availability: "active" },
            ],
          },
          {
            label: "Navigate",
            slots: [
              {
                shortcut: "↑/↓",
                label: "Move",
                availability: available.length > 0 ? "available" : "inactive",
              },
              {
                shortcut: "Enter",
                label: "Run",
                availability: selectedAvailable ? "available" : "inactive",
              },
            ],
          },
          { slots: [{ shortcut: "Esc", label: "Close" }] },
        ]}
      >
      <Box sx={{ px: 1.5, pb: 1.5, pt: 1.25 }}>
        <Box
          sx={{
            minHeight: 44,
            px: 1.5,
            display: "flex",
            alignItems: "center",
            gap: 1.25,
            border: 1,
            borderColor: (theme) => alpha(theme.palette.primary.main, 0.3),
            borderRadius: `${DESKTOP_INSET_RADIUS}px`,
            bgcolor: (theme) => alpha(theme.palette.background.paper, 0.52),
            transition: "border-color 120ms ease, box-shadow 120ms ease, background-color 120ms ease",
            "&:focus-within": {
              borderColor: "primary.main",
              bgcolor: "background.paper",
              boxShadow: (theme) =>
                `0 0 0 2px ${alpha(theme.palette.primary.main, 0.16)}`,
            },
          }}
        >
          <Search
            aria-hidden
            sx={{ flexShrink: 0, color: "text.secondary", fontSize: "1.2rem" }}
          />
          <InputBase
            inputRef={paletteInputRef}
            autoFocus
            fullWidth
            value={query}
            onChange={(event): void => setQuery(event.target.value)}
            onKeyDown={(event): void => {
              if (desktopImeOwnsKey(event.nativeEvent)) return;
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
            inputProps={{ "aria-label": "Search commands" }}
            sx={{
              minWidth: 0,
              fontSize: "0.9rem",
              "& .MuiInputBase-input": {
                p: 0,
                height: "1.5em",
                lineHeight: 1.5,
                caretColor: "primary.main",
                "&::placeholder": { color: "text.secondary", opacity: 0.72 },
              },
            }}
          />
        </Box>
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
                <DesktopShortcut
                  shortcut={command.shortcut}
                  availability={command.when?.() === false ? "inactive" : "available"}
                />
              )}
            </ListItemButton>
          ))}
        </List>
      </Box>
      </DesktopModal>
    </>
  );
}
