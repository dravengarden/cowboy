import { useMemo, useRef } from "react";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./DesktopCommandProvider";
import {
  DESKTOP_WORKSPACE_KEYS,
  desktopLeaderSequence,
} from "./workspaceShortcuts";

export function DesktopPendingEditCommandBindings({
  kind,
  sendable,
  onSlash,
  onReference,
  onAttach,
  onDone,
  onExpand,
}: {
  kind: "queued" | "draft";
  sendable: boolean;
  onSlash: () => void;
  onReference: () => void;
  onAttach: () => void;
  onDone: () => void;
  onExpand: () => void;
}): null {
  const state = useRef({
    sendable,
    onSlash,
    onReference,
    onAttach,
    onDone,
    onExpand,
  });
  state.current = {
    sendable,
    onSlash,
    onReference,
    onAttach,
    onDone,
    onExpand,
  };
  const region = `prompt.${kind}`;
  const prefix = `pendingEdit.${kind}`;
  const commands = useMemo<DesktopCommand[]>(() => [
    {
      id: `${prefix}.slash`,
      title: "Insert slash command",
      group: "Pending message editor",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.composerSlash),
      allowInEditor: true,
      contexts: ["prompt"],
      regions: [region],
      run: () => state.current.onSlash(),
    },
    {
      id: `${prefix}.reference`,
      title: "Reference a file",
      group: "Pending message editor",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.composerReference),
      allowInEditor: true,
      contexts: ["prompt"],
      regions: [region],
      run: () => state.current.onReference(),
    },
    {
      id: `${prefix}.attach`,
      title: "Attach file",
      group: "Pending message editor",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.composerAttach),
      allowInEditor: true,
      contexts: ["prompt"],
      regions: [region],
      run: () => state.current.onAttach(),
    },
    {
      id: `${prefix}.done`,
      title: "Save message changes",
      group: "Pending message editor",
      shortcut: "Mod+S",
      consumeWhenDisabled: true,
      allowInEditor: true,
      contexts: ["prompt"],
      regions: [region],
      when: () => state.current.sendable,
      disabledReason: "The message is empty",
      run: () => state.current.onDone(),
    },
    {
      id: `${prefix}.expand`,
      title: "Expand message editor",
      group: "Pending message editor",
      sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.editorExpand),
      allowInEditor: true,
      contexts: ["prompt"],
      regions: [region],
      run: () => state.current.onExpand(),
    },
  ], [kind]);

  useDesktopCommand(commands[0] as DesktopCommand);
  useDesktopCommand(commands[1] as DesktopCommand);
  useDesktopCommand(commands[2] as DesktopCommand);
  useDesktopCommand(commands[3] as DesktopCommand);
  useDesktopCommand(commands[4] as DesktopCommand);
  return null;
}
