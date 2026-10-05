import { createContext, useContext } from "react";
import type { DesktopHint } from "./hintTargets";

/** `root` lists every leader command in scope; `sessions` turns the next key
 *  into a session label (the `␣␣` switcher); `modal` labels the controls of
 *  the topmost modal; `group:t` holds a group's commands (`␣T…`). */
export type DesktopLeaderLayer =
  | "root"
  | "sessions"
  | "modal"
  /** A which-key group such as `␣T` (Top bar); the key is lower case. */
  | `group:${string}`;

export interface DesktopLeaderState {
  armed: boolean;
  layer: DesktopLeaderLayer;
  open: (layer?: DesktopLeaderLayer) => void;
  close: () => void;
}

// A tiny module so shared surfaces (the Sessions list in App.tsx, keycaps)
// can read the leader without importing the Desktop command provider.
export const DesktopLeaderContext = createContext<DesktopLeaderState | null>(
  null,
);

/** Leader state for which-key and live keycaps; null outside Desktop. */
export function useDesktopLeaderOptional(): DesktopLeaderState | null {
  return useContext(DesktopLeaderContext);
}

/** Hint labels currently on screen; empty when no label layer is armed. */
export const DesktopHintContext = createContext<readonly DesktopHint[]>([]);

export function useDesktopHints(): readonly DesktopHint[] {
  return useContext(DesktopHintContext);
}
