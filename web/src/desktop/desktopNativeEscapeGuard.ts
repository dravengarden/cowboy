import { isImeKeyEvent } from "../imeKey";

export type DesktopEscapeGuardAction =
  | "ignore"
  | "defer-to-editor"
  | "prevent-native";

/**
 * Decide who owns a Desktop Escape before it reaches the native window.
 *
 * CodeMirror/Vim must see an untouched keydown because its keymaps skip events
 * that are already default-prevented. Every other Desktop surface can safely
 * cancel the native default in capture: MUI deliberately ignores
 * `defaultPrevented` for Escape and still closes the top modal. An open modal
 * wins even if focus has momentarily remained inside the editor underneath it.
 */
export function desktopEscapeGuardAction({
  key,
  ime,
  modalOpen,
  editorOwnsEscape,
}: {
  key: string;
  ime: boolean;
  modalOpen: boolean;
  editorOwnsEscape: boolean;
}): DesktopEscapeGuardAction {
  if (key !== "Escape" || ime) return "ignore";
  if (editorOwnsEscape && !modalOpen) return "defer-to-editor";
  return "prevent-native";
}

function editorOwnsEscape(target: EventTarget | null): boolean {
  return target instanceof Element &&
    target.closest(".cm-content, [data-vim-command-sink]") !== null;
}

/**
 * Keep Escape inside Cowboy instead of letting AppKit/native standalone-window
 * handling exit macOS full screen after a component stops propagation.
 *
 * Capture owns the native default for ordinary Desktop UI. The bubble fallback
 * covers editor events after CodeMirror/Vim has had the first chance to process
 * an unmodified keydown. Editor handlers that stop propagation also consume
 * their owned Escape themselves.
 */
export function installDesktopNativeEscapeGuard(): () => void {
  const capture = (event: KeyboardEvent): void => {
    const action = desktopEscapeGuardAction({
      key: event.key,
      ime: isImeKeyEvent(event),
      modalOpen: document.querySelector(".MuiModal-root") !== null,
      editorOwnsEscape: editorOwnsEscape(event.target),
    });
    if (action === "prevent-native") event.preventDefault();
  };
  const bubble = (event: KeyboardEvent): void => {
    if (event.key === "Escape" && !isImeKeyEvent(event)) {
      event.preventDefault();
    }
  };
  globalThis.addEventListener("keydown", capture, true);
  globalThis.addEventListener("keydown", bubble);
  return (): void => {
    globalThis.removeEventListener("keydown", capture, true);
    globalThis.removeEventListener("keydown", bubble);
  };
}
