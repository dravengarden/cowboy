import { useMemo, useRef } from "react";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./DesktopCommandProvider";

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
      shortcut: "Alt+/",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: [region],
      run: () => state.current.onSlash(),
    },
    {
      id: `${prefix}.reference`,
      title: "Reference a file",
      group: "Pending message editor",
      shortcut: "Alt+R",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: [region],
      run: () => state.current.onReference(),
    },
    {
      id: `${prefix}.attach`,
      title: "Attach file",
      group: "Pending message editor",
      shortcut: "Alt+A",
      allowInEditor: true,
      contexts: ["prompt"],
      regions: [region],
      run: () => state.current.onAttach(),
    },
    {
      id: `${prefix}.done`,
      title: "Finish editing message",
      group: "Pending message editor",
      shortcut: "Mod+Enter",
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
      shortcut: "Alt+X",
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
