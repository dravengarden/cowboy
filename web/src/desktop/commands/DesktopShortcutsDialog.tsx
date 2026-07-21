import {
  alpha,
  Box,
  Dialog,
  DialogContent,
  DialogTitle,
  Divider,
  Stack,
  Typography,
} from "@mui/material";
import { Keyboard } from "@mui/icons-material";
import { useMemo } from "react";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { DesktopKeycap, DesktopShortcut } from "./DesktopKeycap";
import { useDesktopCommands } from "./DesktopCommandProvider";

interface ShortcutRow {
  keys: string[];
  title: string;
  description?: string;
}

const NAVIGATION: ShortcutRow[] = [
  { keys: ["Mod+1"], title: "Focus Sessions" },
  { keys: ["Mod+2"], title: "Focus Prompt" },
  { keys: ["Mod+3"], title: "Focus Conversation" },
  { keys: ["Mod+4"], title: "Focus Top Bar" },
  { keys: ["Mod+1…4"], title: "Run config / Usage / Compact / Stop in Top Bar" },
  {
    keys: ["P/O/D/E"],
    title: "Plan / Queue / Drafts / Editor inside Prompt",
    description: "Available from Prompt lists; the composer keeps native Vim commands",
  },
  { keys: ["Ctrl", "W", "H/L"], title: "Move between workspace panes" },
  { keys: ["Ctrl", "W", "J/K"], title: "Move between regions in a pane" },
  { keys: ["Ctrl", "W", "W"], title: "Cycle focus regions" },
  { keys: ["J/K"], title: "Move through items in list regions" },
  { keys: ["Mod+1…0"], title: "Switch directly to a visible session from Sessions" },
  { keys: ["Mod+J/K"], title: "Reorder the focused item when supported" },
  { keys: ["G", "G"], title: "First item" },
  { keys: ["Shift+G"], title: "Last item" },
  { keys: ["Enter"], title: "Open or activate the focused item" },
  { keys: ["I"], title: "Edit the focused item" },
  { keys: ["Esc"], title: "Leave edit mode or close the active layer" },
];

const DISCOVERY: ShortcutRow[] = [
  { keys: ["Mod+/"], title: "Open this shortcut guide from anywhere" },
  { keys: ["Mod+K"], title: "Open Command Palette" },
];

const CONVERSATION: ShortcutRow[] = [
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
    <Stack direction="row" spacing={0.4} alignItems="center" flexWrap="wrap" useFlexGap>
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
    <Dialog
      open={open}
      onClose={onClose}
      fullWidth
      maxWidth="md"
      aria-labelledby="desktop-shortcuts-title"
      slotProps={{
        paper: {
          sx: {
            maxHeight: "min(82vh, 760px)",
            borderRadius: 3,
            border: 1,
            borderColor: (theme) => alpha(theme.palette.primary.main, 0.22),
            backgroundImage: "none",
          },
        },
      }}
    >
      <DialogTitle id="desktop-shortcuts-title" sx={{ pb: 1.25 }}>
        <Stack direction="row" spacing={1.25} alignItems="center">
          <Keyboard color="primary" />
          <Box>
            <Typography variant="h6" fontWeight={750}>Keyboard shortcuts</Typography>
            <Typography variant="body2" color="text.secondary">
              Vim-first navigation · {workspace.focusedRegion ?? workspace.focusedPane}
            </Typography>
          </Box>
          <Box sx={{ flex: 1 }} />
          <DesktopShortcut shortcut="Mod+/" />
        </Stack>
      </DialogTitle>
      <DialogContent dividers sx={{ p: 1.5 }}>
        <Box
          sx={{
            display: "grid",
            gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
            gap: 1.25,
            "@media (max-width: 760px)": { gridTemplateColumns: "1fr" },
          }}
        >
          <ShortcutSection title="Workspace navigation" rows={NAVIGATION} active />
          <ShortcutSection title="Discovery and commands" rows={DISCOVERY} />
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
      </DialogContent>
    </Dialog>
  );
}
