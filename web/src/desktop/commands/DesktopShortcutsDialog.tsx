import {
  alpha,
  Box,
  Divider,
  Stack,
  Typography,
} from "@mui/material";
import { Keyboard } from "@mui/icons-material";
import { useMemo } from "react";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { DesktopKeycap, DesktopShortcut } from "./DesktopKeycap";
import { useDesktopCommands } from "./DesktopCommandProvider";
import {
  DESKTOP_FOCUS_PLAN_SHORTCUT,
  DESKTOP_FOCUS_PROMPT_SHORTCUT,
  DESKTOP_RESIZE_SELECT_SHORTCUT,
  DESKTOP_SESSION_SLOTS_LABEL,
  DESKTOP_SHORTCUTS,
} from "./workspaceShortcuts";
import { DesktopModal } from "../DesktopModal";

interface ShortcutRow {
  keys: string[];
  title: string;
  description?: string;
}

const NAVIGATION: ShortcutRow[] = [
  { keys: [DESKTOP_SHORTCUTS.focusSessions], title: "Focus Sessions / Sidebar" },
  { keys: [DESKTOP_FOCUS_PROMPT_SHORTCUT], title: "Focus Message the Agent" },
  { keys: [DESKTOP_FOCUS_PLAN_SHORTCUT], title: "Focus Plan" },
  { keys: [DESKTOP_SHORTCUTS.focusConversation], title: "Focus Conversation Log" },
  { keys: [DESKTOP_SHORTCUTS.focusTopbar], title: "Focus Top Bar" },
  { keys: [DESKTOP_SHORTCUTS.newSession], title: "Create a new Session" },
  { keys: [DESKTOP_SHORTCUTS.settings], title: "Open Settings" },
  { keys: ["R"], title: "Open Run Configuration in Top Bar" },
  { keys: ["U"], title: "Open Usage Limits in Top Bar" },
  { keys: ["L"], title: "Reload Session Runtime in Top Bar" },
  { keys: ["C"], title: "Compact Conversation in Top Bar" },
  { keys: ["X"], title: "Clear Conversation in Top Bar" },
  {
    keys: [DESKTOP_SHORTCUTS.focusQueue, DESKTOP_SHORTCUTS.focusDrafts],
    title: "Queue / Drafts inside Prompt",
    description:
      `${DESKTOP_FOCUS_PROMPT_SHORTCUT} returns to the editor without changing its Vim mode or caret`,
  },
  { keys: [DESKTOP_SHORTCUTS.cycleRegion], title: "Cycle visible workspace regions" },
  {
    keys: [DESKTOP_RESIZE_SELECT_SHORTCUT],
    title: "Select the nearest layout resize bar",
    description:
      "Enter Resize mode without moving the bar. H/L then resizes, Shift+H/L uses a larger step, Tab selects the next visible bar, and Esc or Enter finishes",
  },
  { keys: ["J/K"], title: "Move through items in list regions" },
  { keys: ["Shift+J/K"], title: "Reorder the focused item when supported" },
  { keys: ["O"], title: "Toggle continuous Order mode for list reordering" },
  { keys: ["G", "G"], title: "First item" },
  { keys: ["Shift+G"], title: "Last item" },
  {
    keys: ["1…0"],
    title: "Jump to a Queue or Draft item while that list is focused",
    description: "Clicking the visible number does the same jump. G then 1–0 remains available",
  },
  {
    keys: ["G", "1…0"],
    title: "Jump to a Queue or Draft item after pressing G",
    description: "The first ten visible rows use 1–9, then 0",
  },
  { keys: ["Enter"], title: "Open or activate the focused item" },
  { keys: ["I"], title: "Edit the focused item" },
  { keys: ["Esc"], title: "Leave the current Vim mode or close the active layer" },
];

const DISCOVERY: ShortcutRow[] = [
  { keys: [DESKTOP_SHORTCUTS.shortcuts], title: "Open this shortcut guide" },
  { keys: [DESKTOP_SHORTCUTS.commands], title: "Open Command Palette" },
];

const DIRECT_ACTIONS: ShortcutRow[] = [
  { keys: ["Mod+Enter"], title: "Send or queue the current prompt" },
  { keys: [DESKTOP_SHORTCUTS.saveDraft], title: "Save the current prompt as a draft" },
  { keys: ["Alt+Enter"], title: "Force push the current prompt" },
  { keys: [DESKTOP_SHORTCUTS.stop], title: "Stop the current turn" },
  {
    keys: [DESKTOP_SESSION_SLOTS_LABEL],
    title: "Switch directly to one of the first ten visible sessions",
  },
];

const CONVERSATION: ShortcutRow[] = [
  { keys: ["Z"], title: "Enter full-screen Reading mode" },
  { keys: ["J/K"], title: "Scroll down / up by one reading line" },
  { keys: ["Ctrl+D/U"], title: "Scroll down / up by half a page" },
  { keys: ["Ctrl+F/B"], title: "Scroll down / up by a page" },
  { keys: ["G", "G"], title: "Jump to the oldest loaded message" },
  { keys: ["Shift+G"], title: "Jump to latest and enable Following" },
  { keys: ["F"], title: "Toggle automatic Following" },
  { keys: ["Tab / Shift+Tab"], title: "Select the next / previous expandable widget" },
  { keys: ["H/L"], title: "Close / open the selected widget" },
  { keys: ["Enter"], title: "Toggle the selected widget" },
  {
    keys: ["A/R"],
    title: "Allow / reject a pending permission",
    description: "Uses the least persistent matching option, preferring once",
  },
];

function KeySequence({ keys }: { keys: string[] }): React.JSX.Element {
  return (
    <Stack
      data-shortcut-reference
      direction="row"
      spacing={0.4}
      alignItems="center"
      flexWrap="wrap"
      useFlexGap
    >
      {keys.map((key, index) =>
        key.includes("+")
          ? <DesktopShortcut key={`${key}-${String(index)}`} shortcut={key} />
          : <DesktopKeycap key={`${key}-${String(index)}`} keyLabel={key} />
      )}
    </Stack>
  );
}

function ShortcutSection({
  title,
  rows,
  active = false,
}: {
  title: string;
  rows: ShortcutRow[];
  active?: boolean;
}): React.JSX.Element {
  return (
    <Box
      sx={{
        border: 1,
        borderColor: (theme) => alpha(
          active ? theme.palette.primary.main : theme.palette.divider,
          active ? 0.34 : 0.72,
        ),
        borderRadius: 2,
        bgcolor: (theme) => alpha(
          active ? theme.palette.primary.main : theme.palette.background.default,
          active ? 0.055 : 0.28,
        ),
        overflow: "hidden",
      }}
    >
      <Typography
        variant="overline"
        color={active ? "primary.main" : "text.secondary"}
        sx={{ display: "block", px: 1.5, py: 1, fontWeight: 750, lineHeight: 1 }}
      >
        {title}
      </Typography>
      <Divider />
      {rows.map((row, index) => (
        <Box
          key={`${row.title}-${String(index)}`}
          sx={{
            minHeight: 46,
            px: 1.5,
            py: 0.8,
            display: "grid",
            gridTemplateColumns: "minmax(118px, auto) 1fr",
            alignItems: "center",
            gap: 1.5,
            borderTop: index === 0 ? 0 : 1,
            borderColor: "divider",
          }}
        >
          <KeySequence keys={row.keys} />
          <Box sx={{ minWidth: 0 }}>
            <Typography variant="body2" fontWeight={650}>{row.title}</Typography>
            {row.description && (
              <Typography variant="caption" color="text.secondary">
                {row.description}
              </Typography>
            )}
          </Box>
        </Box>
      ))}
    </Box>
  );
}

export function DesktopShortcutsDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const registry = useDesktopCommands();
  const paneCommands = useMemo<ShortcutRow[]>(() =>
    registry.commands
      .filter((command) =>
        (command.contexts || command.regions) &&
        (!command.contexts || command.contexts.includes(workspace.focusedPane)) &&
        (!command.regions ||
          (!!workspace.focusedRegion && command.regions.includes(workspace.focusedRegion))) &&
        command.shortcut
      )
      .map((command) => ({
        keys: [command.shortcut as string],
        title: command.title,
        ...(command.description ? { description: command.description } : {}),
      })), [registry.commands, workspace.focusedPane, workspace.focusedRegion]);

  return (
    <DesktopModal
      open={open}
      onClose={onClose}
      title="Keyboard shortcuts"
      description={`Vim-first navigation · ${workspace.focusedRegion ?? workspace.focusedPane}`}
      icon={<Keyboard color="primary" />}
      width={920}
      shortcutGroups={[
        {
          slots: [{
            shortcut: DESKTOP_SHORTCUTS.shortcuts,
            label: "Guide",
            availability: "active",
          }],
        },
        { slots: [{ shortcut: "Esc", label: "Close" }] },
      ]}
    >
      <Box
        sx={{
          p: 1.5,
          display: "grid",
          gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
          gap: 1.25,
          "@media (max-width: 760px)": { gridTemplateColumns: "1fr" },
        }}
      >
        <ShortcutSection title="Workspace navigation" rows={NAVIGATION} active />
        <ShortcutSection title="Discovery and commands" rows={DISCOVERY} />
        <ShortcutSection title="Direct actions" rows={DIRECT_ACTIONS} />
        {workspace.focusedRegion === "conversation.transcript" && (
          <ShortcutSection title="Conversation reading" rows={CONVERSATION} active />
        )}
        {paneCommands.length > 0 && (
          <ShortcutSection
            title={`${workspace.focusedRegion ?? workspace.focusedPane} commands`}
            rows={paneCommands}
            active
          />
        )}
      </Box>
    </DesktopModal>
  );
}
