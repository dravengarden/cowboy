import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
} from "react";
import { isMac } from "../../platform";
import {
  isTextEditingTarget,
  matchesShortcut,
  parseShortcut,
} from "./shortcut";

export interface DesktopCommand {
  id: string;
  title: string;
  shortcut?: string;
  /** Global commands skip text editors unless explicitly opted in. */
  allowInEditor?: boolean;
  when?: () => boolean;
  run: () => void;
}

interface DesktopCommandContextValue {
  register: (command: DesktopCommand) => () => void;
  execute: (id: string) => boolean;
  list: () => DesktopCommand[];
}

const DesktopCommandContext = createContext<DesktopCommandContextValue | null>(
  null,
);

export function DesktopCommandProvider(
  { children }: { children: React.ReactNode },
): React.JSX.Element {
  const commands = useRef(new Map<string, DesktopCommand>());
  const register = useCallback((command: DesktopCommand): () => void => {
    commands.current.set(command.id, command);
    return () => {
      if (commands.current.get(command.id) === command) {
        commands.current.delete(command.id);
      }
    };
  }, []);
  const execute = useCallback((id: string): boolean => {
    const command = commands.current.get(id);
    if (!command || command.when?.() === false) return false;
    command.run();
    return true;
  }, []);
  const value = useMemo<DesktopCommandContextValue>(() => ({
    register,
    execute,
    list: () => [...commands.current.values()],
  }), [execute, register]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.isComposing || event.repeat) return;
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
  }, []);

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
  useEffect(() => registry.register(command), [registry, command]);
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
