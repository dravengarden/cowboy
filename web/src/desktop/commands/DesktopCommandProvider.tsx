import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  createCommandRegistryStore,
  DESKTOP_FIELD_CURSOR_STYLES,
  DesktopCommandContext,
  type DesktopCommandInternalContextValue,
  useDesktopCommand,
  useDesktopCommandActions,
  useDesktopCommands,
  useDesktopCommandSelector,
  useOptionalDesktopCommands,
} from "./commandRegistryStore";
import type { DesktopPane } from "../DesktopWorkspaceController";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { isMac } from "../../platform";
import {
  isTextEditingTarget,
  matchesShortcut,
  parseShortcut,
} from "./shortcut";
import { workspaceCommandKey } from "./workspaceCommandKey";
import { tryHandleTranscriptYank } from "../transcriptYank";
import { assertMacShortcutAllowed } from "./macShortcutPolicy";
import { assertChromeShortcutAllowed } from "./chromeShortcutPolicy";
import { desktopImeOwnsKey } from "./imeShortcut";
import {
  codeMirrorOwnsPrintableKey,
  desktopOverlayOwnsShortcuts,
  remoteViewerOwnsKey,
  swallowLeaderSpaceKeyUp,
} from "./desktopShortcutScope";
import {
  desktopShouldBlockStaleVimSink,
  desktopVimSinkShouldHandleKeys,
} from "../desktopComposerOwnership";
import {
  handlePendingListKey,
  isPendingModEnter,
  listPageTarget,
  listScroller,
  listScrollFor,
  listViewKey,
} from "./listNavigation";
import {
  adjacentDesktopSplitter,
  DESKTOP_SPLITTER_ADJUST_EVENT,
  DESKTOP_SPLITTER_LARGE_STEP,
  DESKTOP_SPLITTER_STEP,
  visibleDesktopSplitterIds,
} from "../desktopSplitterKeyboard";
import type { DesktopSplitterId } from "../DesktopWorkspaceController";
import {
  DESKTOP_LEADER_GROUPS,
  desktopLeaderGroupKey,
  desktopLeaderKey,
  desktopWorkspaceContinuationKey,
  desktopWorkspaceSequenceOwnsKey,
  DESKTOP_WORKSPACE_COMMANDS,
  isDesktopLeaderSpace,
  matchesDesktopWorkspacePrefix,
} from "./workspaceShortcuts";
import { assertShortcutRegistrationAllowed } from "./shortcutRegistrationPolicy";
import { isImeComposing } from "../vim/imeStatusStore";
import { vimSinkAwaitsInput } from "../vim/vimSinkInput";
import { desktopKeyIntent, installNativeCompositionTracker } from "./keyIntent";
import {
  DESKTOP_MOVE_PICK_EVENT,
  DESKTOP_SESSION_JUMP_EVENT,
} from "./sessionJump";
import {
  activateHint,
  type DesktopHint,
  isHintActionable,
  listHints,
  modalHints,
  popupOwnsKeys,
  topmostModal,
} from "./hintTargets";
import { focusAdjacentOutsideRegion, isInteractiveControl } from "./modalGuard";
import { handleDesktopModalKey } from "./modalNavigation";
import { handleDeliveryPickerKey } from "../../deliveryPickerCommands";
import { desktopMenuNavigationKey } from "./menuNavigation";
import {
  lockEditableForLeader,
  unlockEditableForLeader,
} from "./leaderInputLock";
import {
  type RegionDirection,
  regionInDirection,
  regionMotionKey,
} from "./regionNavigation";
import {
  handleInputVimKey,
  installInputVim,
  isInputVimNormal,
} from "../vim/inputVim";
import { GlobalStyles } from "@mui/material";
import {
  DesktopHintContext,
  DesktopLeaderContext,
  type DesktopLeaderLayer,
  type DesktopLeaderState,
} from "./leaderContext";

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
  /** A grouped leader command that runs from any focus (Top bar, an open
   *  Draft); others keep their own pane/region scope inside the group. */
  leaderAnywhere?: boolean;
  /** The surface that owns this key when one letter means different things
   *  on different pages (`␣H`: Schedule in a Session, History in a Draft).
   *  Commands of different surfaces never overlap; only one is mounted. */
  surface?: "session" | "document";
  /** which-key lists the current item's own actions first, under "Here". */
  contextual?: boolean;
  /** The key's resting meaning (`␣1` opens tab 1). Another command may claim
   *  the same sequence and wins whenever it can run (a pending approval). */
  fallback?: boolean;
  /** Reserve a shortcut even while its target is temporarily unavailable. */
  consumeWhenDisabled?: boolean;
  run: () => void;
}

interface PendingJumpChord {
  region: string;
}

const DesktopListJumpContext = createContext<string | null>(null);

/** A which-key group opens only when one of its commands can run now;
 *  otherwise its key keeps its root meaning (`␣D` focuses Drafts unless a
 *  Draft document is open). `leaderAnywhere` commands (Top bar, the open
 *  Draft) run from any focus; the rest (Markup in an editor, Interface in
 *  Prompt) keep their usual scope. */
export function desktopLeaderGroupCommands(
  commands: Iterable<DesktopCommand>,
  group: string,
  focusedPane: DesktopPane,
  focusedRegion: string | null,
): DesktopCommand[] {
  return [...commands].filter((command) => {
    const path = desktopLeaderGroupKey(command);
    return path?.group === group &&
      (!command.contexts || command.contexts.includes(focusedPane)) &&
      (command.leaderAnywhere === true ||
        desktopCommandInScope(command, focusedPane, focusedRegion));
  });
}

export function desktopLeaderGroupAvailable(
  commands: Iterable<DesktopCommand>,
  group: string,
  focusedPane: DesktopPane,
  focusedRegion: string | null,
): boolean {
  return desktopLeaderGroupCommands(
    commands,
    group,
    focusedPane,
    focusedRegion,
  ).some(
    (command) => command.when?.() !== false,
  );
}

/** A page's own root command (`␣R` Title on a Draft) takes its key over a
 *  group of the same letter, if one exists on another page. */
export function desktopSurfaceCommandOwnsKey(
  commands: Iterable<DesktopCommand>,
  key: string,
): boolean {
  for (const command of commands) {
    if (
      command.surface && desktopLeaderKey(command) === key &&
      command.when?.() !== false
    ) return true;
  }
  return false;
}

/**
 * Run a leader continuation's effect after its key has fully landed.
 *
 * Under a CJK input source the key arrives as Process/229, and preventing
 * its keydown cannot stop the input method: after the keydown returns, the
 * IME composes the letter into whatever owns focus *then*. A command that
 * moved focus synchronously (into an editor, or onto the transcript, which
 * forwards typing to the Composer) therefore received the letter: `␣Q` left
 * a "q" behind. While the key is in flight focus stays where the leader was
 * armed (a non-editable sink, chrome, or a field the leader locked), so the
 * composition goes nowhere; the effect runs on that key's keyup.
 */
function afterLeaderKey(event: KeyboardEvent, run: () => void): void {
  if (event.keyCode !== 229 && event.key !== "Process") {
    run();
    return;
  }
  let done = false;
  const finish = (): void => {
    if (done) return;
    done = true;
    globalThis.removeEventListener("keyup", onKeyUp, true);
    globalThis.clearTimeout(timer);
    run();
  };
  const onKeyUp = (up: KeyboardEvent): void => {
    if (up.code === event.code) finish();
  };
  globalThis.addEventListener("keyup", onKeyUp, true);
  const timer = globalThis.setTimeout(finish, 250);
}

/** Where the window motion may take Ctrl+H/J/K/L: everywhere, Vim Insert
 *  and plain text fields included, so moving between regions never needs
 *  a mode change first. Vim gives up its Insert Ctrl-H/J/K (backspace,
 *  newline, digraph) and macOS fields their Emacs Ctrl-H/K; Backspace,
 *  Enter and Cmd/Option+Delete remain. Only a pending Vim command (`d`,
 *  `f`, `r`, …) keeps its next key. */
function regionMotionAllowed(target: EventTarget | null): boolean {
  const element = target instanceof Element ? target : null;
  if (element?.matches("[data-vim-command-sink]")) {
    return !vimSinkAwaitsInput(element);
  }
  return true;
}

function visibleRegions(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>("[data-desktop-region]")]
    .filter((element) => {
      const rect = element.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0 &&
        element.closest("[aria-hidden='true'], [inert]") === null;
    });
}

/** The region nearest the current one in a direction, by on-screen geometry. */
export function desktopRegionInDirection(
  focusedRegion: string | null,
  direction: RegionDirection,
): string | null {
  const regions = visibleRegions();
  const active = document.activeElement;
  const current = (active instanceof Element
    ? active.closest<HTMLElement>("[data-desktop-region]")
    : null) ??
    regions.find((element) => element.dataset.desktopRegion === focusedRegion) ??
    null;
  if (!current) {
    return regions.find((element) =>
      element.dataset.desktopRegion === "prompt.composer"
    )?.dataset.desktopRegion ?? regions[0]?.dataset.desktopRegion ?? null;
  }
  const candidates = regions.filter((element) =>
    element !== current && !element.contains(current) &&
    !current.contains(element) &&
    element.dataset.desktopRegion !== current.dataset.desktopRegion
  );
  const index = regionInDirection(
    current.getBoundingClientRect(),
    candidates.map((element) => element.getBoundingClientRect()),
    direction,
  );
  return index === null ? null : candidates[index]!.dataset.desktopRegion ?? null;
}

/** The scope test shared by dispatch, which-key and live keycaps. */
export function desktopCommandInScope(
  command: DesktopCommand,
  focusedPane: DesktopPane,
  focusedRegion: string | null,
): boolean {
  return (!command.contexts || command.contexts.includes(focusedPane)) &&
    (!command.regions ||
      (!!focusedRegion && command.regions.includes(focusedRegion)));
}

// Sessions tree row commands (FOCUS.md, docs/sessions-folders.md). The row
// decides what each means for a folder versus a session, so the provider only
// names the intent on the focused item.
const SESSIONS_ROW_EVENTS: Readonly<Record<string, string>> = {
  h: "cowboy:desktop-tree-left",
  s: "cowboy:desktop-session-settings",
  m: "cowboy:desktop-session-move",
  n: "cowboy:desktop-folder-new",
  i: "cowboy:desktop-rename",
};

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

function conversationWidgets(region: HTMLElement | null, includeActions = false): HTMLElement[] {
  const selector = includeActions
    ? "[data-desktop-widget-toggle], [data-desktop-transcript-stop], a[href], button, input:not([type='hidden']), select, textarea, summary, [contenteditable='true'], [tabindex]:not([tabindex='-1'])"
    : "[data-desktop-widget-toggle]";
  return [...(region?.querySelectorAll<HTMLElement>(selector) ?? [])]
    .filter((element) => {
      if (element.offsetParent === null || element.matches(":disabled, [aria-disabled='true']")) return false;
      if (element.hasAttribute("data-desktop-transcript-scroller")) return false;
      const marked = element.matches("[data-desktop-widget-toggle], [data-desktop-transcript-stop]");
      if (!marked && element.tabIndex < 0) return false;
      return !element.closest("[aria-hidden='true'], [inert]");
    })
    .sort((left, right) => {
      const a = left.getBoundingClientRect();
      const b = right.getBoundingClientRect();
      return a.top - b.top || a.left - b.left;
    });
}

function nearestConversationWidget(
  region: HTMLElement,
  widgets: HTMLElement[],
  preferMarked = false,
): HTMLElement | null {
  if (widgets.length === 0) return null;
  const scroller = region.querySelector<HTMLElement>("[data-desktop-transcript-scroller]");
  const bounds = scroller?.getBoundingClientRect() ?? region.getBoundingClientRect();
  const pool = preferMarked
    ? widgets.filter((el) => {
      if (!el.matches("[data-desktop-widget-toggle], [data-desktop-transcript-stop]")) return false;
      const r = el.getBoundingClientRect();
      return r.bottom > bounds.top && r.top < bounds.bottom;
    })
    : [];
  const candidates = pool.length > 0 ? pool : widgets;
  const center = bounds.top + bounds.height / 2;
  return candidates.reduce((nearest, candidate) => {
    const candidateRect = candidate.getBoundingClientRect();
    const nearestRect = nearest.getBoundingClientRect();
    const candidateDistance = Math.abs(candidateRect.top + candidateRect.height / 2 - center);
    const nearestDistance = Math.abs(nearestRect.top + nearestRect.height / 2 - center);
    return candidateDistance < nearestDistance ? candidate : nearest;
  });
}

function interceptImeEscape(
  event: KeyboardEvent,
  armed: boolean,
  clearLeader: () => void,
  clearJump: () => void,
): boolean {
  if (event.key !== "Escape" || desktopKeyIntent(event).owner !== "ime") return false;
  if (!event.isComposing) event.stopPropagation();
  if (armed) clearLeader();
  clearJump();
  return true;
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
  const storeRef = useRef<ReturnType<typeof createCommandRegistryStore<DesktopCommand>> | null>(null);
  if (storeRef.current === null) storeRef.current = createCommandRegistryStore<DesktopCommand>();
  const store = storeRef.current;
  const commands = useRef(store.map);
  const itemChord = useRef<number | null>(null);
  // A pending `z` in a list: zz / zt / zb, and za / zM / zR in Sessions.
  const viewChord = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingJumpChord = useRef<PendingJumpChord | null>(null);
  // Leader state. Like which-key it waits for the next key instead of timing
  // out under the reader: Esc, a pointer press, window blur or any completed
  // continuation closes it.
  const leaderArmed = useRef(false);
  const leaderLayerRef = useRef<DesktopLeaderLayer>("root");
  const [leaderLayer, setLeaderLayer] = useState<DesktopLeaderLayer>("root");
  // A Space consumed as the leader must not also activate a focused button
  // on keyup (Firefox activates buttons on Space keyup).
  const swallowSpaceKeyUp = useRef(false);
  const [pendingJumpRegion, setPendingJumpRegion] = useState<string | null>(null);
  // Hint labels on screen (FOCUS.md "Labels"): list `f` rows or the controls
  // of the topmost modal under its leader.
  const hintsRef = useRef<readonly DesktopHint[]>([]);
  const [hints, setHintsState] = useState<readonly DesktopHint[]>([]);
  const setHints = useCallback((next: readonly DesktopHint[]): void => {
    if (hintsRef.current.length === 0 && next.length === 0) return;
    hintsRef.current = next;
    setHintsState(next);
  }, []);
  const workspace = useDesktopWorkspace();
  const workspaceRef = useRef(workspace);
  workspaceRef.current = workspace;
  const clearWorkspaceCommand = useCallback((): void => {
    unlockEditableForLeader();
    leaderArmed.current = false;
    leaderLayerRef.current = "root";
    setLeaderLayer("root");
    if (pendingJumpChord.current === null) setHints([]);
    workspace.setMode("normal");
  }, [setHints, workspace.setMode]);
  const armWorkspaceCommand = useCallback(
    (layer: DesktopLeaderLayer = "root"): void => {
      // Continuation keys must never reach an input method (leaderInputLock).
      lockEditableForLeader();
      leaderArmed.current = true;
      leaderLayerRef.current = layer;
      setLeaderLayer(layer);
      workspace.setMode("command");
    },
    [workspace.setMode],
  );
  const clearPendingJumpChord = useCallback((): void => {
    if (pendingJumpChord.current === null) return;
    pendingJumpChord.current = null;
    setPendingJumpRegion(null);
    if (!leaderArmed.current) setHints([]);
  }, [setHints]);
  // Like the leader, a label chord waits for its key: Esc, a pointer press,
  // focus or mode change, or any unrelated key ends it.
  const armPendingJumpChord = useCallback(
    (region: string, items: readonly HTMLElement[]): void => {
      pendingJumpChord.current = { region };
      setPendingJumpRegion(region);
      setHints(listHints(items));
    },
    [setHints],
  );
  const register = useCallback((command: DesktopCommand): () => void => {
    assertMacShortcutAllowed(command.id, command.shortcut);
    assertChromeShortcutAllowed(command.id, command.shortcut, isMac);
    const prefixStroke = command.sequence?.[0];
    if (prefixStroke) {
      assertMacShortcutAllowed("workspace.prefix", prefixStroke);
      assertChromeShortcutAllowed("workspace.prefix", prefixStroke, isMac);
    }
    assertShortcutRegistrationAllowed(command, store.map.values());
    store.map.set(command.id, command);
    store.notify();
    return () => {
      if (store.map.get(command.id) === command) {
        store.map.delete(command.id);
        store.notify();
      }
    };
  }, [store]);
  const execute = useCallback((
    id: string,
    { anyScope = false }: { anyScope?: boolean } = {},
  ): boolean => {
    const command = store.map.get(id);
    if (!command || command.when?.() === false) return false;
    if (
      !anyScope &&
      !desktopCommandInScope(command, workspaceRef.current.focusedPane, workspaceRef.current.focusedRegion)
    ) return false;
    command.run();
    return true;
  }, [store]);
  const value = useMemo<DesktopCommandInternalContextValue>(() => ({
    register,
    execute,
    list: store.list,
    subscribe: store.subscribe,
    getSnapshot: store.getSnapshot,
  }), [execute, register, store]);

  // Vim Normal/Insert for native text fields follows focus (inputVim).
  useEffect(() => installInputVim(), []);
  // Leader dismissal outside the key path: pointer input and window blur
  // leave the transient layer, as a click outside which-key does.
  useEffect(() => {
    installNativeCompositionTracker();
    const dismiss = (event: Event): void => {
      // which-key entries are clickable; let their click run the command.
      if (
        event.target instanceof Element &&
        event.target.closest("[data-desktop-leader-menu]")
      ) return;
      if (leaderArmed.current) clearWorkspaceCommand();
      clearPendingJumpChord();
      if (event.type === "blur") swallowSpaceKeyUp.current = false; // Released elsewhere.
    };
    globalThis.addEventListener("pointerdown", dismiss, true);
    globalThis.addEventListener("blur", dismiss);
    return () => {
      globalThis.removeEventListener("pointerdown", dismiss, true);
      globalThis.removeEventListener("blur", dismiss);
    };
  }, [clearPendingJumpChord, clearWorkspaceCommand]);

  useEffect(() => {
    const chord = pendingJumpChord.current;
    if (
      chord &&
      (workspace.mode !== "normal" || workspace.focusedRegion !== chord.region)
    ) {
      clearPendingJumpChord();
    }
  }, [clearPendingJumpChord, workspace.focusedRegion, workspace.mode]);

  const handlersRef = useRef({ armPendingJumpChord, armWorkspaceCommand, clearPendingJumpChord, clearWorkspaceCommand, setHints });
  handlersRef.current = { armPendingJumpChord, armWorkspaceCommand, clearPendingJumpChord, clearWorkspaceCommand, setHints };
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (remoteViewerOwnsKey(event)) return;
      const workspace = workspaceRef.current;
      const { armPendingJumpChord, armWorkspaceCommand, clearPendingJumpChord, clearWorkspaceCommand, setHints } = handlersRef.current;
      if (interceptImeEscape(event, leaderArmed.current, clearWorkspaceCommand, clearPendingJumpChord)) return;
      // inputVim owns native text fields (Insert Esc & Normal motions) unless leader/IME is active.
      if (!leaderArmed.current && desktopKeyIntent(event).owner !== "ime" && handleInputVimKey(event)) {
        clearPendingJumpChord();
        return;
      }
      // Window motion (FOCUS.md "Window motion"): Ctrl+H/J/K/L moves to the
      // nearest region in that direction from any command focus. Dialogs run
      // it in their own grammar (modalNavigation).
      const motion = regionMotionKey(event);
      if (
        motion && !leaderArmed.current && !event.repeat &&
        desktopKeyIntent(event).owner !== "ime" &&
        !desktopOverlayOwnsShortcuts(document) &&
        workspace.selectedSplitter === null &&
        regionMotionAllowed(event.target)
      ) {
        event.preventDefault();
        event.stopImmediatePropagation();
        clearPendingJumpChord();
        if (workspace.focusedRegion === "topbar.controls" && motion === "j") {
          workspace.focusAdjacentRegion(1);
          return;
        }
        const next = desktopRegionInDirection(workspace.focusedRegion, motion);
        if (next) workspace.focusRegion(next);
        return;
      }
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
      // A text field in Vim Normal is command chrome: Space arms the leader.
      const textEditorOwnsKey = isTextEditingTarget(event.target) &&
        !isInputVimNormal(event.target) &&
        !(normalCommandSink && !desktopVimSinkShouldHandleKeys({
          targetIsVimSink: normalCommandSink,
          targetRegionFocused: vimSinkRegionFocused,
        }));
      const workspaceSequenceOwnsKey = desktopWorkspaceSequenceOwnsKey(
        event,
        leaderArmed.current,
        isImeComposing(),
      );
      // Menus and listboxes (FOCUS.md "Menus"): Vim letters become the
      // native keys MUI handles, then re-enter here as those keys.
      if (!leaderArmed.current && desktopMenuNavigationKey(event)) {
        clearPendingJumpChord();
        return;
      }
      if (
        desktopKeyIntent(event).owner !== "ime" &&
        handleDeliveryPickerKey(event)
      ) {
        clearPendingJumpChord();
        return;
      }
      // Plain typing in CodeMirror needs neither modal nor workspace navigation.
      if (
        textEditorOwnsKey && !leaderArmed.current && pendingJumpChord.current === null &&
        workspace.selectedSplitter === null && desktopKeyIntent(event).owner === "text" &&
        codeMirrorOwnsPrintableKey(event, commands.current.values(), isMac)
      ) return;
      const overlayOwnsKeys = desktopOverlayOwnsShortcuts(document);
      // Modal navigation accepts physical keys from idle IMEs. Composition
      // and an armed leader have their own handlers.
      if (
        !leaderArmed.current && overlayOwnsKeys &&
        desktopKeyIntent(event).owner !== "ime"
      ) {
        const modal = popupOwnsKeys() ? null : topmostModal();
        if (modal && handleDesktopModalKey(event, modal)) {
          clearPendingJumpChord();
          return;
        }
      }
      // Use the shared composition lifecycle because macOS keydowns can
      // straddle compositionstart/end, even while DOM focus sits on chrome.
      if (
        desktopImeOwnsKey(event) && !workspaceSequenceOwnsKey
      ) {
        if (
          desktopShouldBlockStaleVimSink({
            targetIsVimSink: normalCommandSink,
            targetRegionFocused: vimSinkRegionFocused,
          })
        ) {
          event.stopPropagation();
        }
        if (leaderArmed.current) clearWorkspaceCommand();
        if (itemChord.current !== null) {
          globalThis.clearTimeout(itemChord.current);
          itemChord.current = null;
        }
        clearPendingJumpChord();
        return;
      }
      // A visible configuration popover advertises and owns its own J/K/H/L
      // map. Relinquish the workspace prefix before it consumes those keys.
      if (overlayOwnsKeys) {
        clearPendingJumpChord();
        // A modal has its own leader: every control in it gets a mnemonic
        // label. Menus, listboxes and popovers keep their own keys.
        const modal = popupOwnsKeys() ? null : topmostModal();
        if (modal) {
          const armedHere = leaderArmed.current &&
            leaderLayerRef.current === "modal";
          if (!armedHere) {
            const arm = matchesDesktopWorkspacePrefix(event) ||
              (isDesktopLeaderSpace(event) && !textEditorOwnsKey &&
                desktopKeyIntent(event).owner === "command");
            if (arm) {
              event.preventDefault();
              event.stopImmediatePropagation();
              if (event.code === "Space") swallowSpaceKeyUp.current = true;
              if (!event.repeat) {
                armWorkspaceCommand("modal");
                setHints(modalHints(modal));
              }
              return;
            }
          } else {
            const key = desktopWorkspaceContinuationKey(event);
            if (key !== null && isModifierKey(key)) return;
            if (key === null) {
              clearWorkspaceCommand();
              return;
            }
            event.preventDefault();
            event.stopImmediatePropagation();
            if (event.code === "Space") swallowSpaceKeyUp.current = true;
            if (event.repeat) return;
            const target = hintsRef.current.find((hint) =>
              hint.label === key.toLowerCase() && isHintActionable(hint.element, modal)
            );
            clearWorkspaceCommand();
            if (target) afterLeaderKey(event, () => { if (isHintActionable(target.element, modal)) activateHint(target.element); });
            return;
          }
        }
        if (leaderArmed.current) clearWorkspaceCommand();
        return;
      }
      // Workspace navigation is a browser-safe two-stroke prefix. Capture the
      // prefix before Vim, CodeMirror, or a native input can interpret it; IME
      // and exclusive overlays already had first refusal above.
      if (matchesDesktopWorkspacePrefix(event)) {
        event.preventDefault();
        // This listener lives on Window capture. Stop other listeners on the
        // same node as well as downstream CodeMirror/Vim handlers.
        event.stopImmediatePropagation();
        clearPendingJumpChord();
        if (!event.repeat) armWorkspaceCommand();
        return;
      }
      // Space is the leader wherever Cowboy owns the key: Vim Normal, lists,
      // the reader and chrome. Text fields, toggles (native inputs), IME
      // candidates and auto-repeat keep their native Space.
      // The focused Vim Normal sink is "text-owned" for ordinary keys (Vim
      // runs them), but Space is the leader there exactly as in LazyVim,
      // unless a Vim command is still waiting for its argument.
      const vimNormalLeader = normalCommandSink && vimSinkRegionFocused &&
        !vimSinkAwaitsInput(event.target);
      if (
        !leaderArmed.current && isDesktopLeaderSpace(event) &&
        (!textEditorOwnsKey || vimNormalLeader) &&
        workspace.selectedSplitter === null &&
        desktopKeyIntent(event).owner === "command"
      ) {
        event.preventDefault();
        event.stopImmediatePropagation();
        swallowSpaceKeyUp.current = true;
        clearPendingJumpChord();
        armWorkspaceCommand();
        return;
      }
      if (leaderArmed.current) {
        const key = desktopWorkspaceContinuationKey(event);
        if (key !== null && isModifierKey(key)) return;
        if (key === null) {
          // A distinct modified chord starts a new transaction. Cancel the
          // prefix and let that shortcut continue through the normal matcher.
          clearWorkspaceCommand();
        } else {
          event.preventDefault();
          event.stopImmediatePropagation();
          if (event.code === "Space") swallowSpaceKeyUp.current = true;
          if (event.repeat) return;
          const layer = leaderLayerRef.current;
          // Backspace climbs one which-key layer instead of closing it.
          if (key === "Backspace" && layer !== "root" && layer !== "move") {
            armWorkspaceCommand("root");
            return;
          }
          clearWorkspaceCommand();
          if (key === "Escape") return;
          if (layer === "move") {
            // The Sessions list owns the labels; it keeps the layer open
            // for a label prefix or a key that names no folder.
            const detail = { key, pending: false };
            document.querySelector<HTMLElement>(
              "[data-desktop-region='sessions.list'] ul",
            )?.dispatchEvent(new CustomEvent(DESKTOP_MOVE_PICK_EVENT, { detail }));
            if (detail.pending) armWorkspaceCommand("move");
            return;
          }
          if (layer.startsWith("group:")) {
            // A group's commands run from anywhere: the group itself is the
            // scope, so region-bound actions (Top bar R/U/…) need no focus
            // trip first. Pane contexts and business predicates still apply.
            const group = layer.slice("group:".length);
            const command = desktopLeaderGroupCommands(
              commands.current.values(),
              group,
              workspace.focusedPane,
              workspace.focusedRegion,
            ).find((candidate) =>
              desktopLeaderGroupKey(candidate)?.key === key.toLowerCase()
            );
            if (command && command.when?.() !== false) {
              afterLeaderKey(event, () => command.run());
            }
            return;
          }
          if (layer === "sessions") {
            afterLeaderKey(event, () => {
              const list = document.querySelector<HTMLElement>(
                "[data-desktop-region='sessions.list'] ul",
              );
              const jumped = list !== null && !list.dispatchEvent(
                new CustomEvent(DESKTOP_SESSION_JUMP_EVENT, {
                  cancelable: true,
                  detail: { label: key.toLowerCase() },
                }),
              );
              if (jumped) {
                const target = workspace.collapsedPanes.prompt
                  ? "conversation.transcript"
                  : "prompt.composer";
                requestAnimationFrame(() => workspace.focusRegion(target));
              }
            });
            return;
          }
          // A leader key has one meaning; scoped editors may each register
          // it (`/` in the Composer and in a queued-message editor), and the
          // one owning the current focus runs.
          const leader = key.toLowerCase();
          if (
            DESKTOP_LEADER_GROUPS[leader] &&
            !desktopSurfaceCommandOwnsKey(commands.current.values(), leader) &&
            desktopLeaderGroupAvailable(
              commands.current.values(),
              leader,
              workspace.focusedPane,
              workspace.focusedRegion,
            )
          ) {
            armWorkspaceCommand(`group:${leader}`);
            return;
          }
          const scoped = (command: DesktopCommand): boolean =>
            desktopCommandInScope(
              command,
              workspace.focusedPane,
              workspace.focusedRegion,
            );
          const registered = [...commands.current.values()].filter((command) =>
            desktopLeaderKey(command) === leader
          );
          const fallbackId = DESKTOP_WORKSPACE_COMMANDS[leader];
          const fallback = fallbackId ? commands.current.get(fallbackId) : undefined;
          const runnable = registered.filter((c) =>
            scoped(c) && c.when?.() !== false
          );
          const command =
            runnable.find((c) => c.fallback !== true) ?? runnable[0] ??
              registered.find(scoped) ??
              (fallback && scoped(fallback) ? fallback : undefined);
          if (command && command.when?.() !== false) {
            if (
              workspace.selectedSplitter !== null &&
              command.id !== "workspace.enterResize"
            ) workspace.setSelectedSplitter(null);
            afterLeaderKey(event, () => command.run());
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
      // List labels are a visible, transient `' -> label` chord. Once armed,
      // the next non-modifier key belongs exclusively to it: a shown label
      // moves the cursor to that row, while Escape or an unrelated bare key
      // cancels without leaking through to destructive row actions such as
      // X. A new modified/global chord cancels it but still runs. Moving
      // focus/editor mode clears it without swallowing the new surface's key.
      const pendingChord = pendingJumpChord.current;
      if (pendingChord) {
        const stillOwned = workspace.mode === "normal" &&
          workspace.focusedRegion === pendingChord.region &&
          !textEditorOwnsKey;
        if (!stillOwned) {
          clearPendingJumpChord();
        } else {
          const key = workspaceCommandKey(event);
          const modifierOnly = isModifierKey(key);
          if (modifierOnly) return;
          // Labels are letters, so Shift is case here, not a new chord: a
          // shifted letter must not fall through to Shift+X-style row actions.
          const modified = event.metaKey || event.ctrlKey || event.altKey;
          if (modified) {
            clearPendingJumpChord();
          } else {
            event.preventDefault();
            event.stopPropagation();
            if (event.repeat) return;
            const target = hintsRef.current.find((hint) =>
              hint.label === key.toLowerCase() && isHintActionable(hint.element)
            )?.element;
            clearPendingJumpChord();
            if (target && isHintActionable(target)) {
              target.focus({ preventScroll: true });
              target.scrollIntoView({ block: "nearest" });
            }
            return;
          }
        }
      }
      const region = workspace.focusedRegion
        ? document.querySelector<HTMLElement>(
          `[data-desktop-region="${CSS.escape(workspace.focusedRegion)}"]`,
        )
        : null;
      const items = visibleRegionItems(region);
      const scrollNavigation = region?.dataset.desktopNavigation === "scroll";
      const widgets = scrollNavigation ? conversationWidgets(region) : [];
      const activeEl = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      const activeWidget = activeEl?.closest<HTMLElement>("[data-desktop-widget-toggle]") ?? null;
      const activeActionStop = activeEl?.closest<HTMLElement>("[data-desktop-transcript-stop]") ?? null;
      const activeOtherControl = activeEl !== null && !activeWidget && !activeActionStop && isInteractiveControl(activeEl);
      if (workspace.mode === "normal" && scrollNavigation && region && workspaceCommandKey(event) === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey) {
        const stops = conversationWidgets(region, true);
        const activeStop = (activeEl && stops.includes(activeEl) ? activeEl : null) ?? activeWidget ?? activeActionStop;
        const current = activeStop ? stops.indexOf(activeStop) : -1;
        const nearest = nearestConversationWidget(region, stops, true);
        const next = current >= 0 ? current + (event.shiftKey ? -1 : 1) : (nearest ? stops.indexOf(nearest) : -1);
        const stop = next >= 0 && next < stops.length ? stops[next] : undefined;
        if (stop) {
          event.preventDefault();
          event.stopPropagation();
          stop.focus({ preventScroll: true });
          stop.scrollIntoView({ block: "nearest", inline: "nearest" });
          return;
        }
        if (current >= 0 && focusAdjacentOutsideRegion(region, event.shiftKey ? -1 : 1)) {
          event.preventDefault();
          event.stopPropagation();
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
        // A pending `g` owns the next key (the sequential-chord law): `gg`
        // jumps to the oldest output, and any other unmodified key cancels
        // the chord and is consumed, so `gf` cannot toggle Following and
        // `gl` cannot open a widget. A modified chord (Ctrl-D) cancels it and
        // continues normally; a held modifier alone keeps it armed.
        if (itemChord.current !== null && !isModifierKey(key)) {
          globalThis.clearTimeout(itemChord.current);
          itemChord.current = null;
          if (!event.ctrlKey) {
            event.preventDefault();
            event.stopPropagation();
            if (key !== "g") return;
            action = "oldest";
          }
        }
        if (action === null) {
          if (!event.ctrlKey && !activeActionStop && !activeOtherControl && (key === "h" || key === "l" || key === "Enter")) {
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
          } else if (tryHandleTranscriptYank(event, region, key, activeOtherControl)) {
            return;
          } else if (!event.shiftKey || key === "G") {
            if (key === "g") {
              event.preventDefault();
              itemChord.current = globalThis.setTimeout(() => {
                itemChord.current = null;
              }, 900);
              return;
            }
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
      const pendingList = region?.dataset.desktopRegion === "prompt.queued" ||
        region?.dataset.desktopRegion === "prompt.draft";
      if (
        workspace.mode === "normal" && !textEditorOwnsKey && pendingList &&
        region && (isPendingModEnter(event) || items.length === 0) &&
        handlePendingListKey({
          event,
          key: workspaceCommandKey(event),
          region,
          items,
          active: document.activeElement instanceof HTMLElement
            ? items.indexOf(document.activeElement.closest<HTMLElement>("[data-desktop-item]") as HTMLElement)
            : -1,
          reordering: region.dataset.desktopReordering === "true",
          focusComposer: () => workspace.focusRegion("prompt.composer"),
        })
      ) return;
      const viewMotion = listViewKey(event);
      if (
        workspace.mode === "normal" && !textEditorOwnsKey &&
        (!event.ctrlKey || viewMotion !== null) && !event.metaKey &&
        !event.altKey
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
          if (
            active < 0 &&
            (region?.dataset.desktopRegion === "sessions.list" ||
              region?.dataset.desktopRegion === "sessions.rail")
          ) {
            active = items.findIndex((item) => item.dataset.desktopCurrent === "true");
            if (active < 0) active = 0;
          }
          const sessionsList = region?.dataset.desktopRegion === "sessions.list";
          const pinned = sessionsList && region?.dataset.desktopPinned === "true";
          const reordering = pendingList && region?.dataset.desktopReordering === "true";
          // A pending `g` owns the next key (the sequential-chord law): `gg`
          // jumps to the first row, and any other unmodified key cancels the
          // chord and is consumed before it can reach a row verb such as
          // `s` (send), `x` (remove) or `h` (collapse). A modified chord
          // (Ctrl-D) cancels it and continues normally; a held modifier
          // alone keeps it armed so `g` then `G` still means Shift.
          if (itemChord.current !== null && !isModifierKey(key)) {
            globalThis.clearTimeout(itemChord.current);
            itemChord.current = null;
            if (!event.ctrlKey) {
              event.preventDefault();
              event.stopPropagation();
              if (key === "g") {
                items[0]?.focus({ preventScroll: true });
                items[0]?.scrollIntoView({ block: "nearest" });
              }
              return;
            }
          }
          // Vim's view motions in every list (FOCUS.md "Navigation"):
          // Ctrl-D/U half a page and Ctrl-F/B a page move the cursor and the
          // view together; zz / zt / zb put the cursor row at the centre /
          // top / bottom without moving it.
          if (viewMotion && !reordering && !pinned) {
            event.preventDefault();
            event.stopPropagation();
            const scroller = listScroller(items[0]!);
            const visible = scroller
              ? items.filter((item) => {
                const rect = item.getBoundingClientRect();
                const view = scroller.getBoundingClientRect();
                return rect.bottom > view.top && rect.top < view.bottom;
              }).length
              : items.length;
            const next = listPageTarget(active, items.length, visible, viewMotion);
            if (scroller) {
              const share = viewMotion.startsWith("half") ? 0.5 : 1;
              const direction = viewMotion.endsWith("down") ? 1 : -1;
              scroller.scrollTop += direction * share * scroller.clientHeight;
            }
            items[next]?.focus({ preventScroll: true });
            items[next]?.scrollIntoView({ block: "nearest" });
            return;
          }
          if (viewChord.current !== null && !event.repeat) {
            globalThis.clearTimeout(viewChord.current);
            viewChord.current = null;
            event.preventDefault();
            event.stopPropagation();
            const where = ({ z: "center", t: "top", b: "bottom" } as const)[
              key as "z" | "t" | "b"
            ];
            const row = items[Math.max(0, active)];
            const scroller = row ? listScroller(row) : null;
            if (where && row && scroller) {
              const rect = row.getBoundingClientRect();
              const view = scroller.getBoundingClientRect();
              scroller.scrollTop = listScrollFor(
                where,
                { top: rect.top - view.top, height: rect.height },
                { scrollTop: scroller.scrollTop, height: scroller.clientHeight },
              );
            } else if (sessionsList && (key === "a" || key === "M" || key === "R")) {
              // Vim's folds on the Sessions tree: za the fold button, zM
              // close every folder, zR open every folder.
              region.querySelector<HTMLElement>("ul")?.dispatchEvent(
                new CustomEvent("cowboy:desktop-folders", {
                  cancelable: true,
                  detail: {
                    action: key === "a" ? "fold" : key === "M" ? "collapseAll" : "expandAll",
                    row: null,
                  },
                }),
              );
            }
            return;
          }
          if (key === "z" && !reordering && !pinned && !event.repeat) {
            event.preventDefault();
            event.stopPropagation();
            viewChord.current = globalThis.setTimeout(() => {
              viewChord.current = null;
            }, 1200);
            return;
          }
          // `'` (Vim's mark jump) labels the visible rows of the focused
          // list; the next key moves the cursor there. No row carries a fixed
          // number, and bare `f` stays free (FOCUS.md forbids a hint layer
          // on it).
          if (key === "'" && !reordering && !pinned) {
            event.preventDefault();
            event.stopPropagation();
            if (!event.repeat && region?.dataset.desktopRegion) {
              armPendingJumpChord(region.dataset.desktopRegion, items);
            }
            return;
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
          if (
            pendingList && region && handlePendingListKey({
              event,
              key,
              region,
              items,
              active,
              reordering,
              focusComposer: () => workspace.focusRegion("prompt.composer"),
            })
          ) return;
          const rowEvent = sessionsList && !event.repeat
            ? SESSIONS_ROW_EVENTS[key.toLowerCase()]
            : undefined;
          if (rowEvent) {
            const item = items[active];
            if (item) {
              event.preventDefault();
              event.stopPropagation();
              item.dispatchEvent(
                new CustomEvent(rowEvent, { bubbles: true, cancelable: true }),
              );
              return;
            }
          }
          if (
            sessionsList && items[active]?.dataset.desktopFolderRow === "true" &&
            (key === "Enter" || key.toLowerCase() === "l")
          ) {
            // A folder row expands on `l` and toggles on Enter; it never opens
            // a session or hands focus to the Prompt editor.
            event.preventDefault();
            event.stopPropagation();
            items[active]?.dispatchEvent(
              new CustomEvent("cowboy:desktop-tree-right", {
                bubbles: true,
                cancelable: true,
                detail: { toggle: key === "Enter" },
              }),
            );
            return;
          }
          // Collapsed Sessions rail: folders are items; `l` opens the focused
          // folder's menu (Enter already activates it). The menu then owns
          // j/k/h/l until it closes.
          if (region?.dataset.desktopRegion === "sessions.rail" && !event.repeat) {
            const target = key.toLowerCase() === "l" ? items[active] : undefined;
            if (target) {
              event.preventDefault();
              event.stopPropagation();
              target.focus({ preventScroll: true });
              target.click();
              return;
            }
          }
          let next = -1;
          if (key === "g") {
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
        // This command owns the chord. CodeMirror's DOM handlers do not all
        // consult defaultPrevented (Alt+Enter also means Save Draft there).
        // Do not execute both the workspace action and an editor fallback.
        event.stopImmediatePropagation();
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
    const onKeyUp = swallowLeaderSpaceKeyUp(swallowSpaceKeyUp);
    globalThis.addEventListener("keydown", onKeyDown, true);
    globalThis.addEventListener("keyup", onKeyUp, true);
    return () => {
      globalThis.removeEventListener("keydown", onKeyDown, true);
      globalThis.removeEventListener("keyup", onKeyUp, true);
      if (itemChord.current !== null) {
        globalThis.clearTimeout(itemChord.current);
        itemChord.current = null;
      }
    };
  }, []);

  const leader = useMemo<DesktopLeaderState>(() => ({
    armed: workspace.mode === "command",
    layer: leaderLayer,
    open: (layer = "root") => armWorkspaceCommand(layer),
    close: clearWorkspaceCommand,
  }), [armWorkspaceCommand, clearWorkspaceCommand, leaderLayer, workspace.mode]);

  return (
    <DesktopCommandContext.Provider value={value}>
      <DesktopLeaderContext.Provider value={leader}>
        <DesktopHintContext.Provider value={hints}>
          <DesktopListJumpContext.Provider value={pendingJumpRegion}>
            {/* The modal Normal cursor on a text field (modalNavigation). */}
            <GlobalStyles styles={DESKTOP_FIELD_CURSOR_STYLES} />
            {children}
          </DesktopListJumpContext.Provider>
        </DesktopHintContext.Provider>
      </DesktopLeaderContext.Provider>
    </DesktopCommandContext.Provider>
  );
}

export {
  useDesktopCommand,
  useDesktopCommandActions,
  useDesktopCommands,
  useDesktopCommandSelector,
  useOptionalDesktopCommands,
};

export function useDesktopListJumpChord(region: string): boolean {
  return useContext(DesktopListJumpContext) === region;
}
