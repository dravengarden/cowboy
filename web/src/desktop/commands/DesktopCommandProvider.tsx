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
import { workspaceCommandKey } from "./workspaceCommandKey";

export interface DesktopCommand {
  id: string;
  title: string;
  description?: string;
  group: string;
  shortcut?: string;
  contexts?: DesktopPane[];
  regions?: string[];
  /** Global commands skip text editors unless explicitly opted in. */
  allowInEditor?: boolean | ((target: EventTarget | null) => boolean);
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

function conversationWidgets(region: HTMLElement | null): HTMLElement[] {
  return [...(region?.querySelectorAll<HTMLElement>("[data-desktop-widget-toggle]") ?? [])]
    .filter((element) => element.offsetParent !== null)
    .sort((left, right) => {
      const a = left.getBoundingClientRect();
      const b = right.getBoundingClientRect();
      return a.top - b.top || a.left - b.left;
    });
}

function nearestConversationWidget(
  region: HTMLElement,
  widgets: HTMLElement[],
): HTMLElement | null {
  if (widgets.length === 0) return null;
  const scroller = region.querySelector<HTMLElement>("[data-desktop-transcript-scroller]");
  const bounds = scroller?.getBoundingClientRect() ?? region.getBoundingClientRect();
  const center = bounds.top + bounds.height / 2;
  return widgets.reduce((nearest, candidate) => {
    const candidateRect = candidate.getBoundingClientRect();
    const nearestRect = nearest.getBoundingClientRect();
    const candidateDistance = Math.abs(candidateRect.top + candidateRect.height / 2 - center);
    const nearestDistance = Math.abs(nearestRect.top + nearestRect.height / 2 - center);
    return candidateDistance < nearestDistance ? candidate : nearest;
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
      // Sessions are first-level application navigation, so Mod+1…0 switches
      // them globally instead of being scoped to whichever list currently owns
      // focus. This intentionally works from the composer too: a modifier chord
      // cannot become text, and requiring a preliminary Sessions focus defeats
      // the purpose of a direct workspace switch.
      if (
        workspace.mode === "normal" && mod && !event.altKey && !event.shiftKey &&
        document.querySelector("[role='dialog'], [role='menu']") === null
      ) {
        const match = /^(?:Digit|Numpad)([0-9])$/.exec(event.code);
        if (match?.[1]) {
          const sessionsRegion = document.querySelector<HTMLElement>(
            "[data-desktop-region='sessions.list']",
          );
          const sessions = visibleRegionItems(sessionsRegion);
          const digit = Number(match[1]);
          const session = sessions[digit === 0 ? 9 : digit - 1];
          if (session) {
            event.preventDefault();
            event.stopPropagation();
            session.click();
            if (workspace.focusedRegion === "sessions.list") {
              const id = session.dataset.desktopItem;
              if (id) {
                requestAnimationFrame(() =>
                  sessionsRegion?.querySelector<HTMLElement>(
                    `[data-desktop-item="${CSS.escape(id)}"]`,
                  )?.focus({ preventScroll: true })
                );
              }
            }
            return;
          }
        }
      }
      const widgets = scrollNavigation ? conversationWidgets(region) : [];
      const activeWidget = document.activeElement instanceof HTMLElement
        ? document.activeElement.closest<HTMLElement>("[data-desktop-widget-toggle]")
        : null;
      if (
        workspace.mode === "normal" && scrollNavigation && region &&
        event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey
      ) {
        const current = activeWidget ? widgets.indexOf(activeWidget) : -1;
        const nearest = nearestConversationWidget(region, widgets);
        const nearestIndex = nearest ? widgets.indexOf(nearest) : -1;
        const next = current >= 0
          ? (current + (event.shiftKey ? -1 : 1) + widgets.length) % widgets.length
          : nearestIndex;
        const widget = widgets[next];
        if (widget) {
          event.preventDefault();
          event.stopPropagation();
          widget.focus({ preventScroll: true });
          widget.scrollIntoView({ block: "nearest", inline: "nearest" });
          return;
        }
      }
      // Conversation is a reader, not a selectable event list. Once its region
      // owns focus, standard Vim reading motions scroll the viewport. Expandable
      // widgets add the usual tree semantics: H closes, L opens and Enter toggles
      // the focused (or viewport-nearest) widget. Dispatch scroll movement onto
      // the transcript scroller so its bottom-anchor engine remains authoritative.
      if (
        workspace.mode === "normal" && scrollNavigation &&
        !isTextEditingTarget(event.target) && !event.metaKey && !event.altKey
      ) {
        let action: string | null = null;
        const key = workspaceCommandKey(event);
        if (!event.ctrlKey && (key === "h" || key === "l" || key === "Enter")) {
          const widget = activeWidget ?? (region
            ? nearestConversationWidget(region, widgets)
            : null);
          if (widget) {
            event.preventDefault();
            event.stopPropagation();
            const expanded = widget.getAttribute("aria-expanded") === "true";
            const toggle = key === "Enter" || (key === "h" && expanded) ||
              (key === "l" && !expanded);
            if (toggle) widget.click();
            widget.focus({ preventScroll: true });
            widget.scrollIntoView({ block: "nearest", inline: "nearest" });
            return;
          }
        }
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
      // Reordering remains contextual to the focused list. Positional Mod+number
      // access is reserved globally for Sessions above; local lists use standard
      // Vim j/k, gg/G and Enter instead of overloading the same chord.
      if (
        workspace.mode === "normal" && mod && !event.altKey && !event.shiftKey &&
        !isTextEditingTarget(event.target) &&
        document.querySelector("[role='dialog'], [role='menu']") === null
      ) {
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
        const key = workspaceCommandKey(event);
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
          const opensSession = region?.dataset.desktopRegion === "sessions.list" &&
            (key === "Enter" || key.toLowerCase() === "l");
          if ((key === "Enter" || opensSession) && active >= 0) {
            const item = items[active];
            const action = item?.matches("button,[role='button']")
              ? item
              : item?.querySelector<HTMLElement>("[data-desktop-item-action='default']");
            if (action) {
              event.preventDefault();
              event.stopPropagation();
              action.click();
              if (opensSession) {
                // Let the selected session propagate first, then hand keyboard
                // focus to its Prompt editor. This mirrors Vim's `l`/Enter
                // open semantics instead of leaving focus behind in the rail.
                requestAnimationFrame(() => workspace.focusRegion("prompt.composer"));
              }
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
        const allowedInEditor = typeof command.allowInEditor === "function"
          ? command.allowInEditor(event.target)
          : command.allowInEditor === true;
        if (!allowedInEditor && isTextEditingTarget(event.target)) {
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
