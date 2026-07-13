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
import { isImeComposing } from "../vim/imeStatusStore";

export interface DesktopCommand {
  id: string;
  title: string;
  description?: string;
  group: string;
  leader?: string;
  shortcut?: string;
  contexts?: DesktopPane[];
  regions?: string[];
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
  const windowChord = useRef<number | null>(null);
  const itemChord = useRef<number | null>(null);
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
    if (
      !command || command.when?.() === false ||
      (command.contexts && !command.contexts.includes(workspace.focusedPane)) ||
      (command.regions &&
        (!workspace.focusedRegion || !command.regions.includes(workspace.focusedRegion)))
    ) return false;
    command.run();
    return true;
  }, [workspace.focusedPane, workspace.focusedRegion]);
  const commandList = useMemo(() => [...commands.current.values()], [revision]);
  const value = useMemo<DesktopCommandContextValue>(() => ({
    register,
    execute,
    list: () => [...commands.current.values()],
    commands: commandList,
  }), [commandList, execute, register]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      // Composition is an exclusive native-input transaction. `isComposing`
      // is not reliable for every macOS keydown (the first and final events can
      // straddle compositionstart/end), so consult the shared lifecycle too.
      // Never let a stale Ctrl-W / gg chord move focus while marked text exists.
      if (event.isComposing || isImeComposing() || event.keyCode === 229) {
        if (windowChord.current !== null) {
          globalThis.clearTimeout(windowChord.current);
          windowChord.current = null;
        }
        if (itemChord.current !== null) {
          globalThis.clearTimeout(itemChord.current);
          itemChord.current = null;
        }
        return;
      }
      // Standard Vim window navigation. The first Ctrl-W arms a short chord;
      // the following h/l moves panes, j/k moves vertical regions in the current
      // pane, and w cycles every visible region. Capture-phase handling keeps the
      // same contract while the CM6 editor owns keyboard focus.
      if (windowChord.current !== null) {
        globalThis.clearTimeout(windowChord.current);
        windowChord.current = null;
        const key = event.key.toLowerCase();
        if (["h", "j", "k", "l", "w"].includes(key)) {
          event.preventDefault();
          event.stopPropagation();
          if (key === "h") workspace.focusAdjacentPane(-1);
          else if (key === "l") workspace.focusAdjacentPane(1);
          else if (key === "j") workspace.focusAdjacentRegion(1);
          else if (key === "k") workspace.focusAdjacentRegion(-1);
          else workspace.cycleRegion();
          return;
        }
      }
      if (
        event.ctrlKey && !event.metaKey && !event.altKey &&
        event.key.toLowerCase() === "w"
      ) {
        event.preventDefault();
        event.stopPropagation();
        windowChord.current = globalThis.setTimeout(() => {
          windowChord.current = null;
        }, 1200);
        return;
      }
      if (workspace.mode === "leader") {
        if (event.key === "Escape") {
          // Let the MUI Modal own dismissal so its close path can restore the
          // pre-Leader focus. Exact commands deliberately keep their new focus.
          return;
        }
        if (event.defaultPrevented || event.repeat) return;
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
          command.leader?.split(/\s+/).join("") === next.join("") &&
          (!command.contexts || command.contexts.includes(workspace.focusedPane)) &&
          (!command.regions ||
            (!!workspace.focusedRegion && command.regions.includes(workspace.focusedRegion)))
        );
        const branch = [...commands.current.values()].some((command) =>
          command.leader?.split(/\s+/).join("").startsWith(next.join("")) &&
          (!command.contexts || command.contexts.includes(workspace.focusedPane)) &&
          (!command.regions ||
            (!!workspace.focusedRegion && command.regions.includes(workspace.focusedRegion)))
        );
        if (exact) {
          if (exact.when?.() === false) {
            workspace.setLeaderPrefix(next);
            workspace.setLeaderMessage(typeof exact.disabledReason === "function"
              ? exact.disabledReason()
              : exact.disabledReason ?? "Command is unavailable in the current context");
            return;
          }
          workspace.setLeaderPrefix([]);
          workspace.setLeaderMessage(null);
          workspace.setMode("normal");
          // MUI's focus trap is still mounted during this capture handler. Run
          // the command after Leader unmounts so a focus-jump or newly opened
          // dialog becomes the final owner instead of being pulled back into
          // the command board.
          requestAnimationFrame(() => exact.run());
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
      // Region-local Vim list navigation. It is intentionally inactive inside
      // text editors and native controls; those surfaces keep their own j/k,
      // arrows and IME semantics. Regions expose items through a tiny DOM
      // contract so Sessions, Queue and Drafts share one implementation.
      if (
        workspace.mode === "normal" && !isTextEditingTarget(event.target) &&
        !event.ctrlKey && !event.metaKey && !event.altKey
      ) {
        const key = event.key;
        const region = workspace.focusedRegion
          ? document.querySelector<HTMLElement>(
            `[data-desktop-region="${CSS.escape(workspace.focusedRegion)}"]`,
          )
          : null;
        const items = [...(region?.querySelectorAll<HTMLElement>("[data-desktop-item]") ?? [])]
          .filter((element) => element.offsetParent !== null)
          // DOM order is not always visual order: Transcript deliberately uses
          // column-reverse for stable bottom anchoring. Sort by painted position
          // so j always moves down and k always moves up on screen.
          .sort((left, right) => {
            const a = left.getBoundingClientRect();
            const b = right.getBoundingClientRect();
            return a.top - b.top || a.left - b.left;
          });
        if (items.length > 0) {
          const active = document.activeElement instanceof HTMLElement
            ? items.indexOf(document.activeElement.closest<HTMLElement>("[data-desktop-item]") as HTMLElement)
            : -1;
          let next = -1;
          if (itemChord.current !== null) {
            globalThis.clearTimeout(itemChord.current);
            itemChord.current = null;
            if (key === "g") next = 0;
          } else if (key === "g") {
            event.preventDefault();
            itemChord.current = globalThis.setTimeout(() => {
              itemChord.current = null;
            }, 900);
            return;
          }
          if (key === "j") next = Math.min(items.length - 1, Math.max(0, active + 1));
          else if (key === "k") next = Math.max(0, active < 0 ? items.length - 1 : active - 1);
          else if (key === "G") next = items.length - 1;
          if (next >= 0) {
            event.preventDefault();
            items[next]?.focus({ preventScroll: true });
            items[next]?.scrollIntoView({ block: "nearest" });
            return;
          }
          if (key === "Enter" && active >= 0) {
            const item = items[active];
            const action = item?.matches("button,[role='button']")
              ? item
              : item?.querySelector<HTMLElement>("[data-desktop-item-action='default']");
            if (action) {
              event.preventDefault();
              action.click();
              return;
            }
          }
          if (key.toLowerCase() === "i" && active >= 0) {
            const edit = items[active]?.querySelector<HTMLElement>(
              "[data-desktop-item-action='edit'], button[aria-label='Edit']",
            );
            if (edit) {
              event.preventDefault();
              edit.click();
              return;
            }
          }
        }
      }
      if (event.defaultPrevented || event.repeat) return;
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
        if (
          command.contexts && !command.contexts.includes(workspace.focusedPane)
        ) continue;
        if (
          command.regions &&
          (!workspace.focusedRegion ||
            !command.regions.includes(workspace.focusedRegion))
        ) continue;
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
    globalThis.addEventListener("keydown", onKeyDown, true);
    return () => {
      globalThis.removeEventListener("keydown", onKeyDown, true);
      if (windowChord.current !== null) {
        globalThis.clearTimeout(windowChord.current);
        windowChord.current = null;
      }
      if (itemChord.current !== null) {
        globalThis.clearTimeout(itemChord.current);
        itemChord.current = null;
      }
    };
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
