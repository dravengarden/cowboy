import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
} from "react";
import type { DesktopCommand } from "./DesktopCommandProvider";

/**
 * Microtask-coalesced external store for registered Desktop commands.
 *
 * Keeping command list subscriptions outside React context lets components
 * register and unregister commands on focus or session switches without
 * re-rendering DesktopCommandProvider or walking the entire app fiber tree.
 */
export interface CommandRegistryStore<T> {
  readonly map: Map<string, T>;
  list: () => T[];
  getSnapshot: () => T[];
  subscribe: (listener: () => void) => () => void;
  notify: () => void;
}

export function createCommandRegistryStore<T>(): CommandRegistryStore<T> {
  const map = new Map<string, T>();
  const listeners = new Set<() => void>();
  let snapshot: T[] = [];
  let dirty = false;
  let pendingNotify = false;
  let scheduled = false;

  const rebuild = (): T[] => {
    if (dirty) {
      dirty = false;
      snapshot = [...map.values()];
    }
    return snapshot;
  };

  const flush = (): void => {
    scheduled = false;
    if (!pendingNotify) return;
    pendingNotify = false;
    rebuild();
    for (const listener of listeners) listener();
  };

  const notify = (): void => {
    dirty = true;
    pendingNotify = true;
    if (listeners.size === 0 || scheduled) return;
    scheduled = true;
    queueMicrotask(flush);
  };

  return {
    map,
    list: () => [...map.values()],
    getSnapshot: rebuild,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    notify,
  };
}

export interface DesktopCommandActionsValue {
  register: (command: DesktopCommand) => () => void;
  /** Run a command. A key resolves by focus scope; an explicit pick (the
   *  Command Palette) passes `anyScope` so a Conversation or Top bar command
   *  runs from wherever the palette was opened. `when` still applies. */
  execute: (id: string, options?: { anyScope?: boolean }) => boolean;
  list: () => DesktopCommand[];
}

export interface DesktopCommandContextValue extends DesktopCommandActionsValue {
  commands: DesktopCommand[];
}

export interface DesktopCommandInternalContextValue extends DesktopCommandActionsValue {
  subscribe: (listener: () => void) => () => void;
  getSnapshot: () => DesktopCommand[];
}

export const DesktopCommandContext = createContext<DesktopCommandInternalContextValue | null>(null);

export const DESKTOP_FIELD_CURSOR_STYLES = (theme: {
  palette: { primary: { main: string; contrastText: string } };
}) => ({
  "[data-desktop-field-cursor]": { outline: "none" },
  "[data-desktop-field-cursor]:focus": {
    outline: `2px solid ${theme.palette.primary.main}`,
    outlineOffset: 2,
    borderRadius: 12,
  },
  // Vim Normal in a text field: the one-character selection
  // is the block cursor; no caret blinks beside it.
  "[data-vim-input-mode='normal']": { caretColor: "transparent" },
  "[data-vim-input-mode='normal']::selection": {
    background: theme.palette.primary.main,
    color: theme.palette.primary.contrastText,
  },
});

const EMPTY_COMMANDS: DesktopCommand[] = [];
const NOOP_SUBSCRIBE = (): (() => void) => () => undefined;
const EMPTY_SNAPSHOT = (): DesktopCommand[] => EMPTY_COMMANDS;

export function useDesktopCommand(command: DesktopCommand): void {
  const registry = useContext(DesktopCommandContext);
  if (!registry) {
    throw new Error("useDesktopCommand must be used inside DesktopCommandProvider");
  }
  useEffect(() => registry.register(command), [registry.register, command]);
}

/** Stable command registry actions (`register`, `execute`, `list`) without
 *  subscribing to command-list changes. */
export function useDesktopCommandActions(): DesktopCommandActionsValue {
  const registry = useContext(DesktopCommandContext);
  if (!registry) {
    throw new Error("useDesktopCommandActions must be used inside DesktopCommandProvider");
  }
  return registry;
}

/** Subscribe to a derived slice of registered commands via external store. */
export function useDesktopCommandSelector<T>(
  selector: (commands: readonly DesktopCommand[]) => T,
  equal: (previous: T, next: T) => boolean = Object.is,
): T {
  const registry = useContext(DesktopCommandContext);
  if (!registry) {
    throw new Error("useDesktopCommandSelector must be used inside DesktopCommandProvider");
  }
  const selectorRef = useRef(selector);
  selectorRef.current = selector;
  const equalRef = useRef(equal);
  equalRef.current = equal;
  const cacheRef = useRef<{ value: T } | undefined>(undefined);
  const getSnapshot = useCallback((): T => {
    const next = selectorRef.current(registry.getSnapshot());
    if (cacheRef.current && equalRef.current(cacheRef.current.value, next)) {
      return cacheRef.current.value;
    }
    cacheRef.current = { value: next };
    return next;
  }, [registry]);
  return useSyncExternalStore(registry.subscribe, getSnapshot, getSnapshot);
}

export function useDesktopCommands(): DesktopCommandContextValue {
  const registry = useContext(DesktopCommandContext);
  if (!registry) {
    throw new Error("useDesktopCommands must be used inside DesktopCommandProvider");
  }
  const commands = useSyncExternalStore(
    registry.subscribe,
    registry.getSnapshot,
    registry.getSnapshot,
  );
  return useMemo(
    () => ({
      register: registry.register,
      execute: registry.execute,
      list: registry.list,
      commands,
    }),
    [commands, registry],
  );
}

/** Isolated editor fixtures may omit the Desktop command host. */
export function useOptionalDesktopCommands(): DesktopCommandContextValue | null {
  const registry = useContext(DesktopCommandContext);
  const commands = useSyncExternalStore(
    registry ? registry.subscribe : NOOP_SUBSCRIBE,
    registry ? registry.getSnapshot : EMPTY_SNAPSHOT,
    EMPTY_SNAPSHOT,
  );
  return useMemo(
    () =>
      registry
        ? {
            register: registry.register,
            execute: registry.execute,
            list: registry.list,
            commands,
          }
        : null,
    [commands, registry],
  );
}
