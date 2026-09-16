import { useMemo, useRef } from "react";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./DesktopCommandProvider";
import {
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
  DESKTOP_WORKSPACE_PREFIX,
} from "./workspaceShortcuts";
import { toggleComposerSourceMode } from "../../composerSourceMode";

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
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      run: () => state.current.onSlash(),
    },
    {
      id: "composer.reference",
      title: "Reference a file",
      group: "Prompt actions",
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
      shortcut: DESKTOP_SHORTCUTS.saveDraft,
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
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.canMore,
      disabledReason: "Every prompt action is already visible",
      run: () => state.current.onMore(),
    },
    {
      // Obsidian's "Toggle Live Preview/Source mode", scoped to the Prompt pane
      // because that is where every composer mount lives (the editor, plus the
      // queue and draft edit surfaces). The preference itself is global, so this
      // needs no editor handle and is never disabled — there is no state in
      // which the user may not choose how their own markdown is displayed.
      id: "composer.toggleSourceMode",
      title: "Toggle Source mode",
      description:
        "Show the prompt as literal markdown instead of live preview",
      group: "Prompt actions",
      sequence: [
        DESKTOP_WORKSPACE_PREFIX,
        DESKTOP_WORKSPACE_KEYS.toggleSourceMode,
      ],
      allowInEditor: true,
      contexts: ["prompt"],
      run: () => void toggleComposerSourceMode(),
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
  useDesktopCommand(commands[8] as DesktopCommand);
  return null;
}
