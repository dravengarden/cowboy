import { createContext, useContext } from "react";

/** `root` lists every leader command in scope; `sessions` turns the next key
 *  into a session label (the `␣␣` switcher). */
export type DesktopLeaderLayer = "root" | "sessions";

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
