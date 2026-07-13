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
import { getVimMode } from "../../vimModeStore";
import { getVimSetting } from "../../vimSetting";

export interface DesktopCommand {
  id: string;
  title: string;
  description?: string;
  group: string;
  shortcut?: string;
  contexts?: DesktopPane[];
  regions?: string[];
  /** Global commands skip text editors unless explicitly opted in. */
  allowInEditor?: boolean;
  /** Bare Vim-style command allowed in a text editor only while Vim is Normal. */
  allowInVimNormal?: boolean;
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

function visibleRegionItems(region: HTMLElement | null): HTMLElement[] {
  const horizontal = region?.dataset.desktopAxis === "horizontal";
  return [...(region?.querySelectorAll<HTMLElement>("[data-desktop-item]") ?? [])]
    .filter((element) => element.offsetParent !== null)
    .sort((left, right) => {
      const a = left.getBoundingClientRect();
      const b = right.getBoundingClientRect();
      return horizontal
        ? a.left - b.left || a.top - b.top
        : a.top - b.top || a.left - b.left;
    });
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
      const region = workspace.focusedRegion
        ? document.querySelector<HTMLElement>(
          `[data-desktop-region="${CSS.escape(workspace.focusedRegion)}"]`,
        )
        : null;
      const items = visibleRegionItems(region);
      const scrollNavigation = region?.dataset.desktopNavigation === "scroll";
      const mod = isMac
        ? event.metaKey && !event.ctrlKey
        : event.ctrlKey && !event.metaKey;
      // Conversation is a reader, not a selectable event list. Once its region
      // owns focus, standard Vim reading motions scroll the viewport and F owns
      // the explicit Following state. Dispatch onto the transcript scroller so
      // its existing bottom-anchor engine remains the single scroll authority.
      if (
        workspace.mode === "normal" && scrollNavigation &&
        !isTextEditingTarget(event.target) && !event.metaKey && !event.altKey
      ) {
        let action: string | null = null;
        const key = event.key;
        if (event.ctrlKey) {
          action = ({
            d: "half-page-down",
            u: "half-page-up",
            f: "page-down",
            b: "page-up",
          } as Record<string, string>)[key.toLowerCase()] ?? null;
        } else if (!event.shiftKey || key === "G") {
          if (itemChord.current !== null) {
            globalThis.clearTimeout(itemChord.current);
            itemChord.current = null;
            if (key === "g") action = "oldest";
          } else if (key === "g") {
            event.preventDefault();
            itemChord.current = globalThis.setTimeout(() => {
              itemChord.current = null;
            }, 900);
            return;
          } else {
            action = ({
              j: "line-down",
              k: "line-up",
              G: "latest",
              f: "toggle-following",
            } as Record<string, string>)[key] ?? null;
          }
        }
        if (action) {
          const scroller = region.querySelector<HTMLElement>(
            "[data-desktop-transcript-scroller]",
          );
          if (scroller) {
            event.preventDefault();
            event.stopPropagation();
            scroller.dispatchEvent(new CustomEvent("cowboy:desktop-transcript-nav", {
              detail: { action },
            }));
            return;
          }
        }
      }
      // Item slots are contextual, never global. Once a list region owns focus,
      // Mod+1…0 jumps to its first ten painted rows. This keeps the global key
      // map small while giving every Sessions/Plan/Queue/Drafts/Transcript list
      // the same positional access contract.
      if (
        workspace.mode === "normal" && mod && !event.altKey && !event.shiftKey &&
        !isTextEditingTarget(event.target) &&
        document.querySelector("[role='dialog'], [role='menu']") === null
      ) {
        const match = /^(?:Digit|Numpad)([0-9])$/.exec(event.code);
        if (match?.[1]) {
          const digit = Number(match[1]);
          const item = items[digit === 0 ? 9 : digit - 1];
          if (item) {
            event.preventDefault();
            event.stopPropagation();
            item.focus({ preventScroll: true });
            item.scrollIntoView({ block: "nearest", inline: "nearest" });
            return;
          }
        }
        if (
          region?.dataset.desktopReorderable === "true" &&
          (event.key.toLowerCase() === "j" || event.key.toLowerCase() === "k")
        ) {
          const active = document.activeElement instanceof HTMLElement
            ? document.activeElement.closest<HTMLElement>("[data-desktop-item]")
            : null;
          if (active && items.includes(active)) {
            event.preventDefault();
            event.stopPropagation();
            active.dispatchEvent(new CustomEvent("cowboy:desktop-reorder", {
              bubbles: true,
              detail: { delta: event.key.toLowerCase() === "j" ? 1 : -1 },
            }));
            return;
          }
        }
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
        const horizontal = region?.dataset.desktopAxis === "horizontal";
        if (!scrollNavigation && items.length > 0) {
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
          else if (horizontal && key === "l") {
            next = Math.min(items.length - 1, Math.max(0, active + 1));
          } else if (horizontal && key === "h") {
            next = Math.max(0, active < 0 ? items.length - 1 : active - 1);
          }
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
          const vimNormal = command.allowInVimNormal === true &&
            workspace.mode === "normal" && getVimSetting() &&
            getVimMode() === "normal";
          if (!vimNormal) continue;
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
