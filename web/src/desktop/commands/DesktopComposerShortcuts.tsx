import { useEffect, useMemo, useRef } from "react";
import {
  type DesktopCommand,
  useDesktopCommands,
} from "./DesktopCommandProvider";
import {
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
  desktopLeaderSequence,
} from "./workspaceShortcuts";
import { COMPOSER_COMMANDS } from "../../composerCommands";
import { formatChord } from "./formatChord";
import { toggleComposerSourceMode } from "../../composerSourceMode";

export function DesktopComposerCommandBindings({
  sendable,
  canInsert,
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
  onFormat,
}: {
  sendable: boolean;
  canInsert: boolean;
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
  onFormat: (id: string) => void;
}): null {
  const state = useRef({
    sendable,
    canInsert,
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
    onFormat,
  });
  state.current = {
    sendable,
    canInsert,
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
    onFormat,
  };
  const commands = useMemo<DesktopCommand[]>(() => [
    {
      id: "composer.slash",
      title: "Insert slash command",
      group: "Prompt actions",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.composerSlash),
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.canInsert,
      disabledReason: "Resume this session to use completions",
      run: () => state.current.onSlash(),
    },
    {
      id: "composer.reference",
      title: "Reference a file",
      group: "Prompt actions",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.composerReference),
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.canInsert,
      disabledReason: "Resume this session to use completions",
      run: () => state.current.onReference(),
    },
    {
      id: "composer.attach",
      title: "Attach file",
      description: "Pick an image or file for the current prompt",
      group: "Prompt actions",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.composerAttach),
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
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.composerSchedule),
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
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.composerJumpFront),
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
      consumeWhenDisabled: true,
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.sendable && state.current.canForce,
      disabledReason: "Force push is only available during an active turn",
      run: () => state.current.onForce(),
    },
    {
      id: "composer.more",
      title: "More formatting",
      group: "Prompt actions",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.composerMore),
      allowInEditor: true,
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      when: () => state.current.canMore,
      disabledReason: "Formatting menu is unavailable",
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
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.toggleSourceMode),
      allowInEditor: true,
      contexts: ["prompt"],
      run: () => void toggleComposerSourceMode(),
    },
    ...COMPOSER_COMMANDS.filter((command) =>
      !["slash", "mention", "attach", "sourceMode"].includes(command.id)
    ).map((command): DesktopCommand => ({
      id: `composer.format.${command.id}`,
      title: command.label,
      group: "Prompt formatting",
      allowInEditor: true,
      ...formatChord(command.id),
      contexts: ["prompt"],
      regions: ["prompt.composer"],
      run: () => state.current.onFormat(command.id),
    })),
  ], []);

  const { register } = useDesktopCommands();
  useEffect(() => {
    const unregister = commands.map(register);
    return () => unregister.forEach((remove) => remove());
  }, [commands, register]);
  return null;
}
