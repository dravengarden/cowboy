import { Box } from "@mui/material";
import { useMemo, useRef } from "react";
import { ShortcutKeycap } from "../../ShortcutKeycap";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./DesktopCommandProvider";

export function DesktopContextShortcut({
  badge,
  shortcut,
  children,
}: {
  badge: string;
  shortcut: string;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <Box
      component="span"
      title={shortcut}
      sx={{
        position: "relative",
        display: "inline-flex",
        flexShrink: 0,
        "&:hover .cowboy-context-shortcut, &:focus-within .cowboy-context-shortcut":
          {
            opacity: 1,
            transform: "translateY(0) scale(1)",
          },
        "[data-desktop-focused='true'] & .cowboy-context-shortcut": {
          opacity: 0.82,
          transform: "translateY(0) scale(1)",
        },
      }}
    >
      {children}
      <Box
        className="cowboy-context-shortcut"
        sx={{
          position: "absolute",
          zIndex: 2,
          bottom: -5,
          right: -5,
          display: "inline-flex",
          opacity: 0,
          transform: "translateY(2px) scale(.94)",
          transition: "opacity 120ms ease, transform 120ms ease",
          pointerEvents: "none",
        }}
      >
        <ShortcutKeycap keyLabel={badge} variant="context" />
      </Box>
    </Box>
  );
}

export function DesktopComposerCommandBindings({
  sendable,
  canAttach,
  canJumpFront,
  canForce,
  canMore,
  onSlash,
  onReference,
  onAttach,
  onSaveDraft,
  onSchedule,
  onJumpFront,
  onForce,
  onMore,
}: {
  sendable: boolean;
  canAttach: boolean;
  canJumpFront: boolean;
  canForce: boolean;
  canMore: boolean;
  onSlash: () => void;
  onReference: () => void;
  onAttach: () => void;
  onSaveDraft: () => void;
  onSchedule: () => void;
  onJumpFront: () => void;
  onForce: () => void;
  onMore: () => void;
}): null {
  const state = useRef({
    sendable,
    canAttach,
    canJumpFront,
    canForce,
    canMore,
    onSlash,
    onReference,
    onAttach,
    onSaveDraft,
    onSchedule,
    onJumpFront,
    onForce,
    onMore,
  });
  state.current = {
    sendable,
    canAttach,
    canJumpFront,
    canForce,
    canMore,
    onSlash,
    onReference,
    onAttach,
    onSaveDraft,
    onSchedule,
    onJumpFront,
    onForce,
    onMore,
  };
  const commands = useMemo<DesktopCommand[]>(() => [
    {
      id: "composer.slash",
      title: "Insert slash command",
      group: "Prompt actions",
      leader: "p /",
      shortcut: "Alt+/",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      run: () => state.current.onSlash(),
    },
    {
      id: "composer.reference",
      title: "Reference a file",
      group: "Prompt actions",
      leader: "p r",
      shortcut: "Alt+R",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      run: () => state.current.onReference(),
    },
    {
      id: "composer.attach",
      title: "Attach file",
      description: "Pick an image or file for the current prompt",
      group: "Prompt actions",
      leader: "p a",
      shortcut: "Alt+A",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.canAttach,
      disabledReason: "This session cannot accept attachments",
      run: () => state.current.onAttach(),
    },
    {
      id: "composer.saveDraft",
      title: "Save prompt as draft",
      group: "Prompt actions",
      leader: "p s",
      shortcut: "Mod+S",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.sendable,
      disabledReason: "The composer is empty",
      run: () => state.current.onSaveDraft(),
    },
    {
      id: "composer.schedule",
      title: "Schedule prompt",
      group: "Prompt actions",
      leader: "p t",
      shortcut: "Alt+S",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.sendable,
      disabledReason: "The composer is empty",
      run: () => state.current.onSchedule(),
    },
    {
      id: "composer.jumpFront",
      title: "Jump prompt to front of queue",
      group: "Prompt actions",
      leader: "p j",
      shortcut: "Mod+J",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.sendable && state.current.canJumpFront,
      disabledReason: "No queued messages to jump ahead of",
      run: () => state.current.onJumpFront(),
    },
    {
      id: "composer.forcePush",
      title: "Force push prompt",
      description: "Interrupt the current turn and run this prompt now",
      group: "Prompt actions",
      leader: "p f",
      shortcut: "Alt+Enter",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.sendable && state.current.canForce,
      disabledReason: "Force push is only available during an active turn",
      run: () => state.current.onForce(),
    },
    {
      id: "composer.more",
      title: "Open prompt actions",
      group: "Prompt actions",
      leader: "p m",
      shortcut: "Mod+.",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.canMore,
      disabledReason: "Every prompt action is already visible",
      run: () => state.current.onMore(),
    },
  ], []);

  useDesktopCommand(commands[0] as DesktopCommand);
  useDesktopCommand(commands[1] as DesktopCommand);
  useDesktopCommand(commands[2] as DesktopCommand);
  useDesktopCommand(commands[3] as DesktopCommand);
  useDesktopCommand(commands[4] as DesktopCommand);
  useDesktopCommand(commands[5] as DesktopCommand);
  useDesktopCommand(commands[6] as DesktopCommand);
  useDesktopCommand(commands[7] as DesktopCommand);
  return null;
}
