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
import { useDesktopLeaderOptional } from "./leaderContext";
import { DesktopRecentDialog } from "./DesktopRecentDialog";
import type { DesktopRecentItem } from "../sessionVisits";
import { DesktopShortcutsDialog } from "./DesktopShortcutsDialog";
import { DesktopLeaderMenu } from "./DesktopLeaderMenu";
import { DesktopHintLayer } from "./DesktopHintLayer";
import {
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
  DESKTOP_WORKSPACE_PREFIX,
  desktopLeaderSequence,
} from "./workspaceShortcuts";
import {
  preferredDesktopSplitter,
  visibleDesktopSplitterIds,
} from "../desktopSplitterKeyboard";
import { DESKTOP_INSET_RADIUS } from "../DesktopEmbeddedControl";
import { retrySyncNow } from "../../store";
import { DesktopModal } from "../DesktopModal";
import { desktopImeOwnsKey } from "./imeShortcut";
import {
  DESKTOP_PANES_EXPANDED,
  desktopCollapsedPanesStore,
} from "../../desktopLayout";

function DesktopCommandRegistration(
  { command }: { command: DesktopCommand },
): null {
  useDesktopCommand(command);
  return null;
}

function sessionsListElement(): HTMLElement | null {
  return document.querySelector<HTMLElement>(
    "[data-desktop-region='sessions.list'] ul",
  );
}

/** The Session page (Prompt and Conversation), as opposed to a Draft. */
function sessionWorkspaceMounted(): boolean {
  return document.querySelector("[data-desktop-pane='conversation']") !== null;
}

function sessionsListMounted(): boolean {
  return sessionsListElement() !== null;
}

/** Folder-wide Sessions actions run inside the list (docs/sessions-folders.md);
 *  the focused row, if any, is the subject. */
function dispatchSessionFolders(action: string): void {
  const focused = document.activeElement instanceof HTMLElement
    ? document.activeElement.closest<HTMLElement>("[data-desktop-item]")
      ?.dataset.desktopItem ?? null
    : null;
  sessionsListElement()?.dispatchEvent(
    new CustomEvent("cowboy:desktop-folders", {
      cancelable: true,
      detail: { action, row: focused },
    }),
  );
}

export function DesktopCommandHost({
  onNewSession,
  onOpenSettings,
  recent = [],
  onOpenRecent,
  onRenameSession,
  draftOpen = false,
}: {
  onNewSession: () => void;
  onOpenSettings: () => void;
  /** The jump list without the current item, newest first (sessionVisits). */
  recent?: readonly DesktopRecentItem[];
  onOpenRecent?: (key: string) => void;
  /** Rename the current Session; absent while no Session is open. */
  onRenameSession?: (() => void) | undefined;
  /** A Draft, not a Session, is the workspace item. */
  draftOpen?: boolean;
}): React.JSX.Element {
  const registry = useDesktopCommands();
  const workspace = useDesktopWorkspace();
  const leader = useDesktopLeaderOptional();
  const leaderRef = useRef(leader);
  leaderRef.current = leader;
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [recentOpen, setRecentOpen] = useState(false);
  // Commands are memoized; read the live jump list and handlers at run time.
  const live = useRef({ recent, onOpenRecent, onRenameSession });
  live.current = { recent, onOpenRecent, onRenameSession };
  // The registry changes on every registration; a memo dependency on it
  // would re-register these commands in a loop.
  const registryRef = useRef(registry);
  registryRef.current = registry;
  const openRecent = (key: string): void => {
    live.current.onOpenRecent?.(key);
    // Land where typing continues: the Prompt (a Draft's body is its
    // Prompt region), else the Conversation when Prompt is folded.
    requestAnimationFrame(() =>
      workspace.focusRegion(
        workspace.collapsedPanes.prompt && !key.startsWith("draft:")
          ? "conversation.transcript"
          : "prompt.composer",
      )
    );
  };
  const openRecentRef = useRef(openRecent);
  openRecentRef.current = openRecent;
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
      shortcut: DESKTOP_SHORTCUTS.shortcuts,
      allowInEditor: true,
      run: () => setShortcutsOpen(true),
    },
    {
      id: "commandPalette.open",
      title: "Open Command Palette",
      description: "Search every registered Desktop command",
      group: "Open",
      shortcut: DESKTOP_SHORTCUTS.commands,
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.commandPalette),
      allowInEditor: true,
      run: () => {
        setQuery("");
        setSelected(0);
        setPaletteOpen(true);
      },
    },
    {
      id: "session.switch",
      title: "Switch Session",
      description: "Label every session with a letter, then press it to open",
      group: "Session",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.switchSession),
      when: sessionsListMounted,
      run: () => leaderRef.current?.open("sessions"),
    },
    {
      id: "session.alternate",
      title: "Previous Session or Draft",
      description: "Alt-Tab: return to the item open before this one",
      group: "Session",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.alternateSession),
      when: () => live.current.recent.length > 0 && !!live.current.onOpenRecent,
      run: () => {
        const previous = live.current.recent[0];
        if (previous) openRecentRef.current(previous.key);
      },
    },
    {
      id: "session.recent",
      title: "Recent Sessions and Drafts…",
      description: "The jump list: 1–9 open a row, J/K and Enter choose",
      group: "Session",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.recentSessions),
      when: () => !!live.current.onOpenRecent,
      run: () => setRecentOpen(true),
    },
    {
      // A Draft's name is its title: `␣T` (document.title) goes there.
      id: "item.rename",
      title: "Rename Session",
      description: "Rename the current Session",
      group: "Session",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.rename),
      surface: "session",
      when: () => !!live.current.onRenameSession,
      run: () => live.current.onRenameSession?.(),
    },
    {
      id: "session.new",
      title: "New Session",
      description: "Create a Cowboy session",
      group: "Session",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.newSession),
      run: onNewSession,
    },
    {
      id: "session.folder.new",
      title: "New Session Folder",
      description: "Create a folder in the Sessions sidebar",
      group: "Session",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.sessionsNewFolder),
      when: sessionsListMounted,
      run: () => dispatchSessionFolders("newFolder"),
    },
    {
      id: "session.moveToFolder",
      title: "Move Session to Folder…",
      description: "File the selected (or current) session into a folder",
      group: "Session",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.sessionsMove),
      when: sessionsListMounted,
      run: () => dispatchSessionFolders("move"),
    },
    {
      id: "session.folders.organize",
      title: "Organize Sessions by Project",
      description: "One folder per project; its sessions file themselves",
      group: "Session",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.sessionsOrganize),
      when: sessionsListMounted,
      run: () => dispatchSessionFolders("organize"),
    },
    {
      id: "session.folders.fold",
      title: "Fold Sessions",
      description:
        "The fold button: focus the current session's folder path, expand every folder, or scroll back to the current row",
      group: "Session",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.sessionsFold),
      when: () => sessionsListElement()?.dataset.desktopFoldAction !== undefined,
      run: () => dispatchSessionFolders("fold"),
    },
    {
      id: "session.folders.collapseAll",
      title: "Collapse All Session Folders",
      group: "Session",
      when: sessionsListMounted,
      run: () => dispatchSessionFolders("collapseAll"),
    },
    {
      id: "session.folders.expandAll",
      title: "Expand All Session Folders",
      group: "Session",
      when: sessionsListMounted,
      run: () => dispatchSessionFolders("expandAll"),
    },
    {
      id: "session.reveal",
      title: "Reveal Current Session",
      description: "Expand its folders and focus its row",
      group: "Session",
      when: sessionsListMounted,
      run: () => dispatchSessionFolders("reveal"),
    },
    {
      id: "sync.retry",
      title: "Reconnect Now",
      description: "Retry the Cowboy server connection without waiting",
      group: "Connection",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.reconnect),
      // Offered exactly while a Retry control is on screen.
      when: () => document.querySelector("[data-desktop-sync-retry]") !== null,
      run: retrySyncNow,
    },
    {
      id: "settings.open",
      title: "Open Settings",
      group: "Settings",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.settings),
      run: onOpenSettings,
    },
    {
      id: "workspace.focusTopbar",
      title: "Focus Top Bar",
      description: "Move keyboard focus to session controls and usage",
      group: "Workspace",
      sequence: [
        DESKTOP_WORKSPACE_PREFIX,
        DESKTOP_WORKSPACE_KEYS.focusTopbar,
        DESKTOP_WORKSPACE_KEYS.focusTopbar,
      ],
      when: () => document.querySelector("[data-desktop-region='topbar.controls']") !== null,
      run: () => workspace.focusRegion("topbar.controls"),
    },
    {
      id: "workspace.focusSessions",
      title: "Focus Sessions",
      group: "Workspace",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.focusSessions),
      run: () => workspace.focusPane("sessions"),
    },
    {
      id: "workspace.focusPrompt",
      title: draftOpen ? "Focus Draft" : "Focus Message the Agent",
      description: "Return to the editor without changing its Vim mode or caret",
      group: "Workspace",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.focusPrompt),
      run: () => workspace.focusRegion("prompt.composer"),
    },
    {
      id: "workspace.focusConversation",
      title: "Focus Conversation",
      group: "Workspace",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.focusConversation),
      when: sessionWorkspaceMounted,
      run: () => workspace.focusPane("conversation"),
    },
    {
      id: "workspace.enterResize",
      title: "Select Layout Resize Bar",
      description:
        "Enter Resize mode on the nearest vertical split, then H/L to move it",
      group: "Workspace",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.resize),
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
        );
        if (splitter) {
          workspace.setSelectedSplitter(splitter);
          requestAnimationFrame(() =>
            document.querySelector<HTMLElement>(
              `[data-desktop-splitter="${CSS.escape(splitter)}"]`,
            )?.focus({ preventScroll: true })
          );
        }
      },
    },
    {
      id: "workspace.cycleRegion",
      title: "Cycle Workspace Region",
      description: "Move focus to the next visible Desktop region",
      group: "Workspace",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.cycleRegion),
      run: () => workspace.cycleRegion(),
    },
    {
      id: "workspace.toggleSessions",
      title: workspace.collapsedPanes.sessions ? "Expand Sessions" : "Collapse Sessions",
      description: "Show or hide the Sessions sidebar",
      group: "Workspace",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.toggleSessions),
      run: () => workspace.togglePane("sessions"),
    },
    {
      id: "workspace.togglePrompt",
      title: workspace.collapsedPanes.prompt ? "Expand Prompt" : "Collapse Prompt",
      description: "Show or hide the Prompt column; Conversation takes its width",
      group: "Workspace",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.togglePrompt),
      when: sessionWorkspaceMounted,
      run: () => workspace.togglePane("prompt"),
    },
    {
      id: "workspace.toggleConversation",
      title: workspace.collapsedPanes.conversation
        ? "Expand Conversation"
        : "Collapse Conversation",
      description: "Show or hide the Conversation; Prompt takes its width",
      group: "Workspace",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.toggleConversation),
      when: sessionWorkspaceMounted,
      run: () => workspace.togglePane("conversation"),
    },
    {
      id: "workspace.expandAllPanes",
      title: "Expand All Panes",
      description: "Restore Sessions, Prompt and Conversation",
      group: "Workspace",
      when: () =>
        workspace.collapsedPanes.sessions || workspace.collapsedPanes.prompt ||
        workspace.collapsedPanes.conversation,
      disabledReason: "Every pane is already expanded",
      run: () => desktopCollapsedPanesStore.set(DESKTOP_PANES_EXPANDED),
    },
    {
      id: "prompt.focusPlan",
      title: "Focus Plan",
      description: "Move keyboard focus to the current task plan",
      group: "Prompt",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.focusPlan),
      when: () => document.querySelector("[data-desktop-region='prompt.plan']") !== null,
      disabledReason: "The agent has not published a plan",
      // The prefix continuation is stable even while Plan is absent.
      consumeWhenDisabled: true,
      run: () => workspace.focusRegion("prompt.plan"),
    },
    {
      id: "prompt.focusQueue",
      title: "Open or Close Queue",
      description:
        "Open and focus queued prompts, or close them when the queue already owns focus",
      group: "Prompt",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.focusQueue),
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
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.focusDrafts),
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
    draftOpen,
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
      <DesktopLeaderMenu />
      <DesktopHintLayer />
      <DesktopShortcutsDialog
        open={shortcutsOpen}
        onClose={(): void => setShortcutsOpen(false)}
      />
      <DesktopRecentDialog
        open={recentOpen}
        items={recent}
        onClose={(): void => setRecentOpen(false)}
        onOpen={(key): void => openRecentRef.current(key)}
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
              {
                shortcut: DESKTOP_SHORTCUTS.commands,
                label: "Palette",
                availability: "active",
              },
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
            // A launcher: typing is all it is for, so one Esc closes it
            // (FOCUS.md "Modals"), as in every command palette.
            inputProps={{
              "aria-label": "Search commands",
              "data-desktop-escape": "close",
              "data-desktop-vim": "off",
            }}
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
              {(command.shortcut || command.sequence) && (
                <DesktopShortcut
                  shortcut={command.shortcut ?? command.sequence?.join(" → ") ?? ""}
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
