import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { DesktopPane } from "../DesktopWorkspaceController";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { isMac } from "../../platform";
import {
  isTextEditingTarget,
  matchesShortcut,
  parseShortcut,
} from "./shortcut";

export interface DesktopCommand {
  id: string;
  title: string;
  description?: string;
  group: string;
  leader?: string;
  shortcut?: string;
  contexts?: DesktopPane[];
  /** Global commands skip text editors unless explicitly opted in. */
  allowInEditor?: boolean;
  when?: () => boolean;
  disabledReason?: string | (() => string);
  run: () => void;
}

interface DesktopCommandContextValue {
  register: (command: DesktopCommand) => () => void;
  execute: (id: string) => boolean;
  list: () => DesktopCommand[];
  commands: DesktopCommand[];
}

const DesktopCommandContext = createContext<DesktopCommandContextValue | null>(
  null,
);

function ownsSpaceKey(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest(
    "button, a, input, textarea, select, [contenteditable='true'], [role='button'], [role='menuitem'], [role='option']",
  ) !== null;
}

export function DesktopCommandProvider(
  { children }: { children: React.ReactNode },
): React.JSX.Element {
  const commands = useRef(new Map<string, DesktopCommand>());
  const [revision, setRevision] = useState(0);
  const workspace = useDesktopWorkspace();
  const register = useCallback((command: DesktopCommand): () => void => {
    commands.current.set(command.id, command);
    setRevision((value) => value + 1);
    return () => {
      if (commands.current.get(command.id) === command) {
        commands.current.delete(command.id);
        setRevision((value) => value + 1);
      }
    };
  }, []);
  const execute = useCallback((id: string): boolean => {
    const command = commands.current.get(id);
    if (!command || command.when?.() === false) return false;
    command.run();
    return true;
  }, []);
  const commandList = useMemo(() => [...commands.current.values()], [revision]);
  const value = useMemo<DesktopCommandContextValue>(() => ({
    register,
    execute,
    list: () => [...commands.current.values()],
    commands: commandList,
  }), [commandList, execute, register]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.isComposing || event.repeat) return;
      if (workspace.mode === "leader") {
        if (event.key === "Escape") {
          event.preventDefault();
          workspace.setLeaderPrefix([]);
          workspace.setLeaderMessage(null);
          workspace.setMode("normal");
          return;
        }
        if (event.key === "Backspace") {
          event.preventDefault();
          workspace.setLeaderPrefix(workspace.leaderPrefix.slice(0, -1));
          workspace.setLeaderMessage(null);
          return;
        }
        if (event.key.length !== 1 || event.ctrlKey || event.metaKey || event.altKey) return;
        event.preventDefault();
        const next = [...workspace.leaderPrefix, event.key.toLowerCase()];
        const exact = [...commands.current.values()].find((command) =>
          command.leader?.split(/\s+/).join("") === next.join("")
        );
        const branch = [...commands.current.values()].some((command) =>
          command.leader?.split(/\s+/).join("").startsWith(next.join(""))
        );
        if (exact) {
          if (exact.when?.() === false) {
            workspace.setLeaderPrefix(next);
            workspace.setLeaderMessage(typeof exact.disabledReason === "function"
              ? exact.disabledReason()
              : exact.disabledReason ?? "Command is unavailable in the current context");
            return;
          }
          exact.run();
          workspace.setLeaderPrefix([]);
          workspace.setLeaderMessage(null);
          workspace.setMode("normal");
          return;
        }
        if (branch) {
          workspace.setLeaderPrefix(next);
          workspace.setLeaderMessage(null);
        } else {
          workspace.setLeaderMessage(`No command for SPC ${next.join(" ")}`);
        }
        return;
      }
      if (
        event.key === " " && !event.ctrlKey && !event.metaKey && !event.altKey &&
        !isTextEditingTarget(event.target) && !ownsSpaceKey(event.target)
      ) {
        event.preventDefault();
        workspace.setLeaderPrefix([]);
        workspace.setLeaderMessage(null);
        workspace.setMode("leader");
        return;
      }
      for (const command of commands.current.values()) {
        if (!command.shortcut || command.when?.() === false) continue;
        if (!command.allowInEditor && isTextEditingTarget(event.target)) {
          continue;
        }
        if (!matchesShortcut(parseShortcut(command.shortcut), event, isMac)) {
          continue;
        }
        event.preventDefault();
        command.run();
        return;
      }
    };
    globalThis.addEventListener("keydown", onKeyDown);
    return () => globalThis.removeEventListener("keydown", onKeyDown);
  }, [workspace]);

  return (
    <DesktopCommandContext.Provider value={value}>
      {children}
    </DesktopCommandContext.Provider>
  );
}

export function useDesktopCommand(command: DesktopCommand): void {
  const registry = useContext(DesktopCommandContext);
  if (!registry) {
    throw new Error(
      "useDesktopCommand must be used inside DesktopCommandProvider",
    );
  }
  useEffect(() => registry.register(command), [registry.register, command]);
}

export function useDesktopCommands(): DesktopCommandContextValue {
  const registry = useContext(DesktopCommandContext);
  if (!registry) {
    throw new Error(
      "useDesktopCommands must be used inside DesktopCommandProvider",
    );
  }
  return registry;
}
