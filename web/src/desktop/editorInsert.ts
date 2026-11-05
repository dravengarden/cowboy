import { useMemo, useRef } from "react";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./commands/DesktopCommandProvider";
import {
  DESKTOP_WORKSPACE_KEYS,
  desktopLeaderSequence,
} from "./commands/workspaceShortcuts";

/**
 * `␣I`: the Prompt's editor, ready to type at its caret. The command only
 * knows the region; a Vim editor in it answers this event (imeAutoInsertVim)
 * and cancels it, entering Insert as Vim's `i`. Without Vim the editor is
 * focused directly: there, focus is already Insert.
 */
export const DESKTOP_EDITOR_INSERT_EVENT = "cowboy:desktop-editor-insert";

const PROMPT_REGION = "[data-desktop-region='prompt.composer']";

export function requestDesktopEditorInsert(): void {
  const region = document.querySelector<HTMLElement>(PROMPT_REGION);
  if (!region) return;
  const handled = !region.dispatchEvent(
    new CustomEvent(DESKTOP_EDITOR_INSERT_EVENT, {
      bubbles: true,
      cancelable: true,
    }),
  );
  if (!handled) region.querySelector<HTMLElement>(".cm-content")?.focus();
}

/** Registers `␣I` beside `␣P` (Focus Prompt, which keeps the Vim mode). */
export function useDesktopInsertCommand(
  draftOpen: boolean,
  focusRegion: (region: string) => void,
): void {
  const focus = useRef(focusRegion);
  focus.current = focusRegion;
  const command = useMemo<DesktopCommand>(() => ({
    id: "workspace.insertPrompt",
    title: draftOpen ? "Write in Draft" : "Write to the Agent",
    description: "Focus the editor at its caret in Insert mode, as Vim's i",
    group: "Workspace",
    sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.insertPrompt),
    run: () => {
      focus.current("prompt.composer");
      // The region may be expanding or swapping editors this frame.
      requestAnimationFrame(requestDesktopEditorInsert);
    },
  }), [draftOpen]);
  useDesktopCommand(command);
}
