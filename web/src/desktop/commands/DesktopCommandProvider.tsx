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
import { workspaceCommandKey } from "./workspaceCommandKey";
import { assertMacShortcutAllowed } from "./macShortcutPolicy";
import { desktopImeOwnsKey } from "./imeShortcut";
import { desktopOverlayOwnsShortcuts } from "./desktopShortcutScope";
import { listJumpIndex, pendingItemActionKey } from "./listNavigation";

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
    assertMacShortcutAllowed(command.id, command.shortcut);
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
      const eventElement = event.target instanceof Element ? event.target : null;
      const normalCommandSink = eventElement?.matches("[data-vim-command-sink]") ?? false;
      // A pane pointer/focus transition can update the workspace region one
      // frame before DOM focus leaves the Composer's hidden Normal-mode sink.
      // In that brief mismatch the newly focused non-editor region owns the
      // physical Vim key; the sink is not a text field and must not strand list
      // navigation. Real editors remain exclusive.
      const textEditorOwnsKey = isTextEditingTarget(event.target) &&
        !(normalCommandSink && workspace.focusedRegion !== "prompt.composer");
      // Composition is an exclusive native-input transaction. `isComposing`
      // is not reliable for every macOS keydown (the first and final events can
      // straddle compositionstart/end), so consult the shared lifecycle too.
      // Never let a stale Ctrl-W / gg chord move focus while marked text exists.
      // A CJK input source can still label a physical Normal-mode key as
      // `Process`/229 even though the non-editable command sink cannot own a
      // composition. Let that sink continue through the physical-key command
      // path; a real shared IME transaction remains exclusive.
      if (
        desktopImeOwnsKey(event)
      ) {
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
      // A visible configuration popover advertises and owns its own J/K/H/L
      // map. Relinquish the workspace map before it consumes those keys.
      if (desktopOverlayOwnsShortcuts(document)) return;
      if (workspace.productMode === "reading") {
        const key = workspaceCommandKey(event);
        if (!event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
          if (key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            workspace.setProductMode("agent");
            requestAnimationFrame(() => workspace.focusRegion("conversation.transcript"));
          } else if (key.toLowerCase() === "p") {
            event.preventDefault();
            event.stopPropagation();
            const closing = workspace.readingSidebarOpen;
            workspace.setReadingSidebarOpen(!closing);
            if (closing) {
              requestAnimationFrame(() => workspace.focusRegion("conversation.transcript"));
            }
          }
        }
        const scroller = document.querySelector<HTMLElement>(
          "[data-desktop-product-mode='reading'] [data-desktop-transcript-scroller]",
        );
        let readingAction: string | null = null;
        if (!event.metaKey && !event.altKey && !isTextEditingTarget(event.target)) {
          if (event.ctrlKey) {
            readingAction = ({
              d: "half-page-down",
              u: "half-page-up",
              f: "page-down",
              b: "page-up",
            } as Record<string, string>)[key.toLowerCase()] ?? null;
          } else if (!event.shiftKey || key === "G") {
            if (itemChord.current !== null) {
              globalThis.clearTimeout(itemChord.current);
              itemChord.current = null;
              if (key === "g") readingAction = "oldest";
            } else if (key === "g") {
              event.preventDefault();
              itemChord.current = globalThis.setTimeout(() => {
                itemChord.current = null;
              }, 900);
              return;
            } else {
              readingAction = ({
                j: "line-down",
                k: "line-up",
                G: "latest",
                F: "toggle-following",
              } as Record<string, string>)[key] ?? null;
            }
          }
        }
        if (readingAction && scroller) {
          event.preventDefault();
          event.stopPropagation();
          scroller.dispatchEvent(new CustomEvent("cowboy:desktop-transcript-nav", {
            detail: { action: readingAction },
          }));
        }
        // Reading owns an isolated command domain. Unhandled keys continue to
        // the reading surface (native selection/find/copy and Explore's [ ]
        // paging), but never enter Agent's pane, queue, draft, or session map.
        return;
      }
      // Direct Vim window movement. Keep Ctrl-W + motion below for users who
      // prefer the canonical two-stroke form, while Ctrl-H/J/K/L provides the
      // fast one-stroke path between Desktop regions. Resolve through
      // `workspaceCommandKey`, never `event.key`, so a CJK input source cannot
      // translate or hide the physical navigation keys. An active composition
      // has already returned above and remains exclusively owned by the IME.
      if (
        event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey
      ) {
        const key = workspaceCommandKey(event).toLowerCase();
        if (["h", "j", "k", "l"].includes(key)) {
          event.preventDefault();
          event.stopPropagation();
          if (key === "h") workspace.focusAdjacentPane(-1);
          else if (key === "l") workspace.focusAdjacentPane(1);
          else if (key === "j") workspace.focusAdjacentRegion(1);
          else workspace.focusAdjacentRegion(-1);
          return;
        }
      }
      // Standard Vim window navigation. The first Ctrl-W arms a short chord;
      // the following h/l moves panes, j/k moves vertical regions in the current
      // pane, and w cycles every visible region. Capture-phase handling keeps the
      // same contract while the CM6 editor owns keyboard focus.
      if (windowChord.current !== null) {
        globalThis.clearTimeout(windowChord.current);
        windowChord.current = null;
        const key = workspaceCommandKey(event).toLowerCase();
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
        workspaceCommandKey(event).toLowerCase() === "w"
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
      // directly by slot from anywhere in the workspace.
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
            const id = session.dataset.desktopItem;
            session.click();
            // Positional session selection also enters the Sessions keyboard
            // region. The row remains the active list item for J/K, while
            // L/Enter explicitly opens its Prompt editor. Previously this only
            // happened when Sessions already owned focus, leaving Mod+number
            // visually selected but keyboard focus stranded in another pane.
            workspace.focusRegion("sessions.list");
            if (id) {
              requestAnimationFrame(() =>
                sessionsRegion?.querySelector<HTMLElement>(
                  `[data-desktop-item="${CSS.escape(id)}"]`,
                )?.focus({ preventScroll: true })
              );
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
        workspaceCommandKey(event) === "Tab" && !event.ctrlKey &&
        !event.metaKey && !event.altKey
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
              F: "toggle-following",
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
        !textEditorOwnsKey &&
        document.querySelector("[role='dialog'], [role='menu']") === null
      ) {
        if (
          region?.dataset.desktopReorderable === "true" &&
          (["j", "k"].includes(workspaceCommandKey(event).toLowerCase()))
        ) {
          const active = document.activeElement instanceof HTMLElement
            ? document.activeElement.closest<HTMLElement>("[data-desktop-item]")
            : null;
          if (active && items.includes(active)) {
            event.preventDefault();
            event.stopPropagation();
            active.dispatchEvent(new CustomEvent("cowboy:desktop-reorder", {
              bubbles: true,
              detail: { delta: workspaceCommandKey(event).toLowerCase() === "j" ? 1 : -1 },
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
        workspace.mode === "normal" && !textEditorOwnsKey &&
        !event.ctrlKey && !event.metaKey && !event.altKey
      ) {
        const key = workspaceCommandKey(event);
        const horizontal = region?.dataset.desktopAxis === "horizontal";
        if (!scrollNavigation && items.length > 0) {
          let active = document.activeElement instanceof HTMLElement
            ? items.indexOf(document.activeElement.closest<HTMLElement>("[data-desktop-item]") as HTMLElement)
            : -1;
          // Region focus can arrive one render before DOM focus reaches the row.
          // Sessions still have an authoritative current item, so anchor Vim
          // navigation there instead of treating the list as selection-less.
          if (active < 0 && region?.dataset.desktopRegion === "sessions.list") {
            active = items.findIndex((item) => item.dataset.desktopCurrent === "true");
            if (active < 0) active = 0;
          }
          const sessionsList = region?.dataset.desktopRegion === "sessions.list";
          const pinned = sessionsList && region?.dataset.desktopPinned === "true";
          const pendingList = region?.dataset.desktopRegion === "prompt.queued" ||
            region?.dataset.desktopRegion === "prompt.draft";
          const reordering = pendingList && region?.dataset.desktopReordering === "true";
          if (sessionsList && key.toLowerCase() === "p" && !event.repeat) {
            event.preventDefault();
            event.stopPropagation();
            region.querySelector<HTMLElement>("ul")?.dispatchEvent(
              new CustomEvent("cowboy:desktop-toggle-pin"),
            );
            items[active]?.focus({ preventScroll: true });
            return;
          }
          if (sessionsList && pinned && key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            region.querySelector<HTMLElement>("ul")?.dispatchEvent(
              new CustomEvent("cowboy:desktop-release-pin"),
            );
            return;
          }
          if (sessionsList && pinned && (key === "j" || key === "k")) {
            event.preventDefault();
            event.stopPropagation();
            items[active]?.dispatchEvent(new CustomEvent("cowboy:desktop-reorder", {
              bubbles: true,
              detail: { delta: key === "j" ? 1 : -1 },
            }));
            return;
          }
          if (pendingList && key.toLowerCase() === "p" && !event.repeat) {
            event.preventDefault();
            event.stopPropagation();
            region.querySelector<HTMLElement>("[data-desktop-pending-list]")?.dispatchEvent(
              new CustomEvent("cowboy:desktop-toggle-reorder"),
            );
            items[Math.max(0, active)]?.focus({ preventScroll: true });
            return;
          }
          if (pendingList && reordering && key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            region.querySelector<HTMLElement>("[data-desktop-pending-list]")?.dispatchEvent(
              new CustomEvent("cowboy:desktop-release-reorder"),
            );
            return;
          }
          if (pendingList && reordering && (key === "j" || key === "k")) {
            event.preventDefault();
            event.stopPropagation();
            items[Math.max(0, active)]?.dispatchEvent(new CustomEvent("cowboy:desktop-reorder", {
              bubbles: true,
              detail: { delta: key === "j" ? 1 : -1 },
            }));
            return;
          }
          if (sessionsList && key.toLowerCase() === "h" && !event.repeat) {
            const item = items[active];
            if (item) {
              event.preventDefault();
              event.stopPropagation();
              item.dispatchEvent(new CustomEvent("cowboy:desktop-session-settings", {
                bubbles: true,
              }));
              return;
            }
          }
          let next = -1;
          if (itemChord.current !== null) {
            globalThis.clearTimeout(itemChord.current);
            itemChord.current = null;
            const jump = listJumpIndex(key, items.length);
            if (key === "g" || pendingList) next = jump ?? -1;
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
          if (pendingList && active >= 0) {
            const pendingAction = pendingItemActionKey(key);
            const action = pendingAction
              ? items[active]?.querySelector<HTMLElement>(
                `[data-desktop-item-action='${pendingAction}']`,
              )
              : null;
            if (action) {
              event.preventDefault();
              event.stopPropagation();
              action.click();
              return;
            }
          }
          const opensSession = region?.dataset.desktopRegion === "sessions.list" &&
            (key === "Enter" || key.toLowerCase() === "l");
          const opensPending =
            (region?.dataset.desktopRegion === "prompt.queued" ||
              region?.dataset.desktopRegion === "prompt.draft") &&
            (key === "Enter" || key.toLowerCase() === "l");
          if ((key === "Enter" || opensSession || opensPending) && active >= 0) {
            const item = items[active];
            const action = opensPending
              ? item?.querySelector<HTMLElement>(
                "[data-desktop-item-action='edit'], button[aria-label='Edit']",
              )
              : item?.matches("button,[role='button']")
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
      // Region/context commands intentionally shadow a global command using the
      // same chord when the focused surface owns a more specific action.
      const rankedCommands = [...commands.current.values()].sort((left, right) =>
        Number(Boolean(right.regions || right.contexts)) -
        Number(Boolean(left.regions || left.contexts))
      );
      for (const command of rankedCommands) {
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
        // Bare contextual commands are physical Vim-style keys only outside a
        // text editor or on the Normal-mode command sink. Never apply this
        // fallback to the editable surface: Insert mode must keep native text.
        const physicalBare = normalCommandSink || !isTextEditingTarget(event.target);
        if (
          !matchesShortcut(
            parseShortcut(command.shortcut),
            event,
            isMac,
            physicalBare,
          )
        ) {
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
