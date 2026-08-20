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
import { assertChromeShortcutAllowed } from "./chromeShortcutPolicy";
import { desktopImeOwnsKey } from "./imeShortcut";
import { desktopOverlayOwnsShortcuts } from "./desktopShortcutScope";
import {
  desktopShouldBlockStaleVimSink,
  desktopVimSinkShouldHandleKeys,
} from "../desktopComposerOwnership";
import { listJumpIndex, pendingItemActionKey } from "./listNavigation";
import {
  adjacentDesktopSplitter,
  DESKTOP_SPLITTER_ADJUST_EVENT,
  DESKTOP_SPLITTER_LARGE_STEP,
  DESKTOP_SPLITTER_STEP,
  visibleDesktopSplitterIds,
} from "../desktopSplitterKeyboard";
import type { DesktopSplitterId } from "../DesktopWorkspaceController";
import {
  desktopWorkspaceContinuationKey,
  DESKTOP_WORKSPACE_COMMANDS,
  matchesDesktopWorkspacePrefix,
} from "./workspaceShortcuts";
import { assertShortcutRegistrationAllowed } from "./shortcutRegistrationPolicy";

export interface DesktopCommand {
  id: string;
  title: string;
  description?: string;
  group: string;
  shortcut?: string;
  /** Ordered strokes for a prefix command. Sequences are not direct shortcuts. */
  sequence?: readonly string[];
  contexts?: DesktopPane[];
  regions?: string[];
  /** Global commands skip text editors unless explicitly opted in. */
  allowInEditor?: boolean | ((target: EventTarget | null) => boolean);
  when?: () => boolean;
  disabledReason?: string | (() => string);
  /** Reserve a shortcut even while its target is temporarily unavailable. */
  consumeWhenDisabled?: boolean;
  run: () => void;
}

interface DesktopCommandContextValue {
  register: (command: DesktopCommand) => () => void;
  execute: (id: string) => boolean;
  list: () => DesktopCommand[];
  commands: DesktopCommand[];
}

interface PendingJumpChord {
  region: string;
  timer: number;
}

const DesktopCommandContext = createContext<DesktopCommandContextValue | null>(
  null,
);
const DesktopListJumpContext = createContext<string | null>(null);

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

function isModifierKey(key: string): boolean {
  return [
    "Alt",
    "AltGraph",
    "CapsLock",
    "Control",
    "Fn",
    "FnLock",
    "Meta",
    "Shift",
  ].includes(key);
}

export function DesktopCommandProvider(
  { children }: { children: React.ReactNode },
): React.JSX.Element {
  const commands = useRef(new Map<string, DesktopCommand>());
  const itemChord = useRef<number | null>(null);
  const pendingJumpChord = useRef<PendingJumpChord | null>(null);
  const workspaceCommandTimer = useRef<number | null>(null);
  const [revision, setRevision] = useState(0);
  const [pendingJumpRegion, setPendingJumpRegion] = useState<string | null>(null);
  const workspace = useDesktopWorkspace();
  const clearWorkspaceCommand = useCallback((): void => {
    if (workspaceCommandTimer.current !== null) {
      globalThis.clearTimeout(workspaceCommandTimer.current);
      workspaceCommandTimer.current = null;
    }
    workspace.setMode("normal");
  }, [workspace.setMode]);
  const armWorkspaceCommand = useCallback((): void => {
    if (workspaceCommandTimer.current !== null) {
      globalThis.clearTimeout(workspaceCommandTimer.current);
    }
    workspace.setMode("command");
    workspaceCommandTimer.current = globalThis.setTimeout(() => {
      workspaceCommandTimer.current = null;
      workspace.setMode("normal");
    }, 2000);
  }, [workspace.setMode]);
  const clearPendingJumpChord = useCallback((): void => {
    const chord = pendingJumpChord.current;
    if (chord) globalThis.clearTimeout(chord.timer);
    pendingJumpChord.current = null;
    setPendingJumpRegion((current) => current === null ? current : null);
  }, []);
  const armPendingJumpChord = useCallback((region: string): void => {
    const current = pendingJumpChord.current;
    if (current) globalThis.clearTimeout(current.timer);
    const timer = globalThis.setTimeout(() => {
      if (pendingJumpChord.current?.timer !== timer) return;
      pendingJumpChord.current = null;
      setPendingJumpRegion((armed) => armed === region ? null : armed);
    }, 1200);
    pendingJumpChord.current = { region, timer };
    setPendingJumpRegion(region);
  }, []);
  const register = useCallback((command: DesktopCommand): () => void => {
    assertMacShortcutAllowed(command.id, command.shortcut);
    assertChromeShortcutAllowed(command.id, command.shortcut, isMac);
    const prefixStroke = command.sequence?.[0];
    if (prefixStroke) {
      assertMacShortcutAllowed("workspace.prefix", prefixStroke);
      assertChromeShortcutAllowed("workspace.prefix", prefixStroke, isMac);
    }
    assertShortcutRegistrationAllowed(command, commands.current.values());
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
    return () => {
      if (workspaceCommandTimer.current !== null) {
        globalThis.clearTimeout(workspaceCommandTimer.current);
        workspaceCommandTimer.current = null;
      }
    };
  }, []);

  useEffect(() => {
    const chord = pendingJumpChord.current;
    if (
      chord &&
      (workspace.productMode !== "agent" || workspace.mode !== "normal" ||
        workspace.focusedRegion !== chord.region)
    ) {
      clearPendingJumpChord();
    }
  }, [clearPendingJumpChord, workspace.focusedRegion, workspace.mode, workspace.productMode]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      const eventElement = event.target instanceof Element ? event.target : null;
      const normalCommandSink = eventElement?.matches("[data-vim-command-sink]") ?? false;
      const vimSinkRegionFocused = normalCommandSink
        ? eventElement?.closest<HTMLElement>("[data-desktop-region]")
          ?.dataset.desktopFocused === "true"
        : false;
      // A pane pointer/focus transition can update the workspace region one
      // frame before DOM focus leaves the Composer's hidden Normal-mode sink.
      // React commits the region's `data-desktop-focused` state before this
      // effect installs its next closure. Trust that DOM ownership marker so a
      // newly opened Queue/Draft editor does not lose its first `i` to the stale
      // list keymap, while a sink left behind in an unfocused region remains inert.
      const textEditorOwnsKey = isTextEditingTarget(event.target) &&
        !(normalCommandSink && !desktopVimSinkShouldHandleKeys({
          targetIsVimSink: normalCommandSink,
          targetRegionFocused: vimSinkRegionFocused,
        }));
      // Composition is an exclusive native-input transaction. `isComposing`
      // is not reliable for every macOS keydown (the first and final events can
      // straddle compositionstart/end), so consult the shared lifecycle and
      // native text-service markers too. Those markers remain IME-owned even
      // when modal focus currently sits on non-editable chrome. Never let a
      // stale Ctrl-W / gg chord move focus while marked text exists. The sole
      // exception is the non-editable Vim Normal sink when no real shared
      // composition exists; it deliberately receives physical Vim commands.
      if (
        desktopImeOwnsKey(event)
      ) {
        if (
          desktopShouldBlockStaleVimSink({
            targetIsVimSink: normalCommandSink,
            targetRegionFocused: vimSinkRegionFocused,
          })
        ) {
          event.stopPropagation();
        }
        if (workspaceCommandTimer.current !== null) clearWorkspaceCommand();
        if (itemChord.current !== null) {
          globalThis.clearTimeout(itemChord.current);
          itemChord.current = null;
        }
        clearPendingJumpChord();
        return;
      }
      // A visible configuration popover advertises and owns its own J/K/H/L
      // map. Relinquish the workspace prefix before it consumes those keys.
      if (desktopOverlayOwnsShortcuts(document)) {
        clearPendingJumpChord();
        if (workspaceCommandTimer.current !== null) clearWorkspaceCommand();
        return;
      }
      // Workspace navigation is a browser-safe two-stroke prefix. Capture the
      // prefix before Vim, CodeMirror, or a native input can interpret it; IME
      // and exclusive overlays already had first refusal above.
      if (matchesDesktopWorkspacePrefix(event)) {
        event.preventDefault();
        event.stopPropagation();
        clearPendingJumpChord();
        if (!event.repeat) armWorkspaceCommand();
        return;
      }
      if (workspaceCommandTimer.current !== null) {
        const key = desktopWorkspaceContinuationKey(event);
        if (key !== null && isModifierKey(key)) return;
        if (key === null) {
          // A distinct modified chord starts a new transaction. Cancel the
          // prefix and let that shortcut continue through the normal matcher.
          clearWorkspaceCommand();
        } else {
          event.preventDefault();
          event.stopPropagation();
          if (event.repeat) return;
          clearWorkspaceCommand();
          if (key === "Escape") return;
          const commandId = DESKTOP_WORKSPACE_COMMANDS[key.toLowerCase()];
          if (!commandId) return;
          const command = commands.current.get(commandId);
          const inContext = command &&
            (!command.contexts || command.contexts.includes(workspace.focusedPane)) &&
            (!command.regions || (!!workspace.focusedRegion &&
              command.regions.includes(workspace.focusedRegion)));
          if (inContext && command.when?.() !== false) {
            if (workspace.productMode !== "agent") workspace.setProductMode("agent");
            if (
              workspace.selectedSplitter !== null &&
              commandId !== "workspace.enterResize"
            ) workspace.setSelectedSplitter(null);
            command.run();
          }
          return;
        }
      }
      const selectSplitter = (splitter: DesktopSplitterId | null): void => {
        if (splitter === null) return;
        workspace.setSelectedSplitter(splitter);
        requestAnimationFrame(() =>
          document.querySelector<HTMLElement>(
            `[data-desktop-splitter="${CSS.escape(splitter)}"]`,
          )?.focus({ preventScroll: true })
        );
      };
      const selectSessionSlot = (digit: string): void => {
        const sessionsRegion = document.querySelector<HTMLElement>(
          "[data-desktop-region='sessions.list']",
        );
        const sessions = visibleRegionItems(sessionsRegion);
        const slot = Number(digit);
        const session = sessions[slot === 0 ? 9 : slot - 1];
        if (!session) return;
        const id = session.dataset.desktopItem;
        session.click();
        workspace.focusRegion("sessions.list");
        if (id) {
          requestAnimationFrame(() =>
            sessionsRegion?.querySelector<HTMLElement>(
              `[data-desktop-item="${CSS.escape(id)}"]`,
            )?.focus({ preventScroll: true })
          );
        }
      };
      if (workspace.selectedSplitter !== null) {
        const visible = visibleDesktopSplitterIds();
        const splitter = document.querySelector<HTMLElement>(
          `[data-desktop-splitter="${CSS.escape(workspace.selectedSplitter)}"]`,
        );
        if (!splitter || !visible.includes(workspace.selectedSplitter)) {
          workspace.setSelectedSplitter(null);
          return;
        }
        const key = workspaceCommandKey(event);
        if (!event.metaKey && !event.ctrlKey && !event.altKey) {
          if (key === "Escape" || key === "Enter") {
            event.preventDefault();
            event.stopPropagation();
            workspace.setSelectedSplitter(null);
            if (workspace.focusedRegion) {
              requestAnimationFrame(() =>
                workspace.focusRegion(workspace.focusedRegion as string)
              );
            }
            return;
          }
          if (key === "Tab") {
            event.preventDefault();
            event.stopPropagation();
            selectSplitter(adjacentDesktopSplitter(
              visible,
              workspace.selectedSplitter,
              event.shiftKey ? -1 : 1,
            ));
            return;
          }
          const lower = key.toLowerCase();
          if (lower === "h" || lower === "l" || key === "ArrowLeft" || key === "ArrowRight") {
            event.preventDefault();
            event.stopPropagation();
            const left = lower === "h" || key === "ArrowLeft";
            const step = event.shiftKey
              ? DESKTOP_SPLITTER_LARGE_STEP
              : DESKTOP_SPLITTER_STEP;
            globalThis.dispatchEvent(new CustomEvent(
              DESKTOP_SPLITTER_ADJUST_EVENT,
              {
                detail: {
                  splitter: workspace.selectedSplitter,
                  delta: left ? -step : step,
                },
              },
            ));
            return;
          }
          // Resize mode is exclusive: unrelated bare keys must not leak into
          // the selected list, transcript widgets, or destructive row actions.
          event.preventDefault();
          event.stopPropagation();
          return;
        }
      }
      // Queue and Draft direct jumps are a visible, transient `G -> slot`
      // chord. Once armed, the next non-modifier key belongs exclusively to
      // that chord: a valid 1-9/0 (or a second G) jumps, while Escape, a
      // unrelated bare key cancels without leaking through to destructive row
      // actions such as X. A new modified/global chord cancels G but remains
      // available (for example Alt+1 switches sessions). Moving focus/editor
      // mode clears the chord without swallowing the new surface's first key.
      const pendingChord = pendingJumpChord.current;
      if (pendingChord) {
        const stillOwned = workspace.productMode === "agent" &&
          workspace.mode === "normal" &&
          workspace.focusedRegion === pendingChord.region &&
          !textEditorOwnsKey;
        if (!stillOwned) {
          clearPendingJumpChord();
        } else {
          const key = workspaceCommandKey(event);
          const modifierOnly = isModifierKey(key);
          if (modifierOnly) return;
          const modified = event.metaKey || event.ctrlKey || event.altKey ||
            event.shiftKey;
          if (modified) {
            clearPendingJumpChord();
          } else {
            event.preventDefault();
            event.stopPropagation();
            if (event.repeat) return;
            clearPendingJumpChord();
            const region = document.querySelector<HTMLElement>(
              `[data-desktop-region="${CSS.escape(pendingChord.region)}"]`,
            );
            const items = visibleRegionItems(region);
            const jump = listJumpIndex(key, items.length);
            if (jump !== null) {
              items[jump]?.focus({ preventScroll: true });
              items[jump]?.scrollIntoView({ block: "nearest" });
            }
            return;
          }
        }
      }
      // Sessions are first-level application navigation. Alt/Option+1…0 is
      // browser-safe on every Desktop platform and works from Insert and
      // Reading modes as well as ordinary workspace regions.
      if (
        event.altKey && !event.metaKey &&
        !event.ctrlKey && !event.shiftKey
      ) {
        const match = /^(?:Digit|Numpad)([0-9])$/.exec(event.code);
        if (match?.[1]) {
          event.preventDefault();
          event.stopPropagation();
          selectSessionSlot(match[1]);
          return;
        }
      }
      if (workspace.productMode === "reading") {
        const key = workspaceCommandKey(event);
        const readingSidebarOwnsKey = Boolean(eventElement?.closest(
          "[data-reading-question-sidebar]",
        ));
        if (!event.metaKey && !event.ctrlKey && !event.altKey) {
          const product = key.toLowerCase();
          if (key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            workspace.setProductMode("agent");
            requestAnimationFrame(() => workspace.focusRegion("conversation.transcript"));
          } else if (product === "p") {
            event.preventDefault();
            event.stopPropagation();
            const closing = workspace.readingSidebarOpen;
            workspace.setReadingSidebarOpen(!closing);
            if (closing) {
              requestAnimationFrame(() => workspace.focusRegion("conversation.transcript"));
            }
          } else if (product === "v") {
            event.preventDefault();
            event.stopPropagation();
            commands.current.get("conversation.toggleProjection")?.run();
          } else if (product === "f") {
            event.preventDefault();
            event.stopPropagation();
            document.querySelector<HTMLButtonElement>(
              "[data-desktop-product-mode='reading'] [data-desktop-conversation-follow]",
            )?.click();
          }
        }
        // The docked question directory is its own Vim list. Its J/K, gg/G,
        // Ctrl-D/U, L/Enter and H bindings must reach PageList instead of
        // scrolling the transcript behind it. Reading-level Esc/P/V/F above
        // remain available from either side of the workspace.
        if (readingSidebarOwnsKey) return;
        const scroller = document.querySelector<HTMLElement>(
          "[data-desktop-product-mode='reading'] [data-desktop-transcript-scroller]",
        );
        let readingAction: string | null = null;
        if (!event.metaKey && !event.altKey && !textEditorOwnsKey) {
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
      const region = workspace.focusedRegion
        ? document.querySelector<HTMLElement>(
          `[data-desktop-region="${CSS.escape(workspace.focusedRegion)}"]`,
        )
        : null;
      const items = visibleRegionItems(region);
      const scrollNavigation = region?.dataset.desktopNavigation === "scroll";
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
        !textEditorOwnsKey && !event.metaKey && !event.altKey
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
        } else if (key.toLowerCase() === "f") {
          action = "toggle-following";
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
      // Reordering remains contextual to the focused list. Shift+J/K is a
      // direct Vim-style variation and avoids Chrome's Ctrl+J/K commands.
      if (
        workspace.mode === "normal" && event.shiftKey && !event.altKey &&
        !event.ctrlKey && !event.metaKey &&
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
          if (pendingList && key === "g") {
            event.preventDefault();
            event.stopPropagation();
            if (!event.repeat && region?.dataset.desktopRegion) {
              armPendingJumpChord(region.dataset.desktopRegion);
            }
            return;
          }
          if (pendingList && !reordering && /^[0-9]$/.test(key)) {
            const jump = listJumpIndex(key, items.length);
            if (jump !== null) {
              event.preventDefault();
              event.stopPropagation();
              items[jump]?.focus({ preventScroll: true });
              items[jump]?.scrollIntoView({ block: "nearest" });
              return;
            }
          }
          if (sessionsList && key.toLowerCase() === "o" && !event.repeat) {
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
          if (pendingList && key.toLowerCase() === "o" && !event.repeat) {
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
          if (!pendingList) {
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
      // Registration rejects overlapping direct chords, so command behavior is
      // stable and independent of component mount order.
      for (const command of commands.current.values()) {
        if (!command.shortcut) continue;
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
        if (!allowedInEditor && textEditorOwnsKey) {
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
        const disabled = command.when?.() === false;
        if (disabled && !command.consumeWhenDisabled) continue;
        event.preventDefault();
        if (!disabled) command.run();
        return;
      }
      // Conversation/top-bar chrome can be highlighted while the hidden Prompt
      // sink still has DOM focus. Stop the event here so `i`/`a`/IME cannot
      // pop Prompt into Insert.
      if (
        desktopShouldBlockStaleVimSink({
          targetIsVimSink: normalCommandSink,
          targetRegionFocused: vimSinkRegionFocused,
        })
      ) {
        event.stopPropagation();
      }
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    return () => {
      globalThis.removeEventListener("keydown", onKeyDown, true);
      if (itemChord.current !== null) {
        globalThis.clearTimeout(itemChord.current);
        itemChord.current = null;
      }
      if (pendingJumpChord.current !== null) {
        globalThis.clearTimeout(pendingJumpChord.current.timer);
        pendingJumpChord.current = null;
      }
    };
  }, [
    armPendingJumpChord,
    armWorkspaceCommand,
    clearPendingJumpChord,
    clearWorkspaceCommand,
    workspace,
  ]);

  return (
    <DesktopCommandContext.Provider value={value}>
      <DesktopListJumpContext.Provider value={pendingJumpRegion}>
        {children}
      </DesktopListJumpContext.Provider>
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

export function useDesktopListJumpChord(region: string): boolean {
  return useContext(DesktopListJumpContext) === region;
}
