export type RunConfigDirectShortcut = "a" | "m" | "e" | "c" | "f" | "p";

export type RunConfigKeyAction =
  | { type: "field"; delta: -1 | 1 }
  | { type: "choice"; delta: -1 | 1 }
  | { type: "pane"; delta: -1 | 1; wrap: boolean }
  | { type: "edge"; edge: "first" | "last" }
  | { type: "go" }
  | { type: "preset"; index: number }
  | { type: "direct"; shortcut: RunConfigDirectShortcut };

/** Resolve standard arrows, Vim motions and the visible mnemonic keys.
 *  `G` is a Vim motion and keeps its case; product letters ignore it. */
export function runConfigKeyAction(key: string): RunConfigKeyAction | null {
  if (key === "G") return { type: "edge", edge: "last" };
  if (key === "g") return { type: "go" };
  switch (key.toLowerCase()) {
    case "k":
    case "arrowup":
      return { type: "field", delta: -1 };
    case "j":
    case "arrowdown":
      return { type: "field", delta: 1 };
    case "h":
    case "arrowleft":
      return { type: "choice", delta: -1 };
    case "l":
    case "arrowright":
      return { type: "choice", delta: 1 };
    case "[":
      return { type: "pane", delta: -1, wrap: true };
    case "]":
      return { type: "pane", delta: 1, wrap: true };
    case "a":
    case "m":
    case "e":
    case "c":
    case "f":
    case "p":
      return {
        type: "direct",
        shortcut: key.toLowerCase() as RunConfigDirectShortcut,
      };
    default:
      break;
  }
  const digit = /^[1-9]$/.exec(key);
  return digit ? { type: "preset", index: Number(key) - 1 } : null;
}

/** Window motion (`Ctrl+H/J/K/L`) inside the panel: across panes sideways,
 *  fields vertically, as it moves between regions outside a dialog. */
export function runConfigMotionAction(
  motion: "h" | "j" | "k" | "l",
): RunConfigKeyAction {
  switch (motion) {
    case "h":
      return { type: "pane", delta: -1, wrap: false };
    case "l":
      return { type: "pane", delta: 1, wrap: false };
    case "j":
      return { type: "field", delta: 1 };
    case "k":
      return { type: "field", delta: -1 };
  }
}

/** Pick the next enabled choice, clamping navigation and wrapping direct cycles. */
export function nextRunConfigChoiceIndex(
  length: number,
  current: number,
  delta: -1 | 1,
  wrap: boolean,
): number {
  if (length <= 0) return -1;
  const safeCurrent = current >= 0 && current < length ? current : 0;
  if (wrap) return (safeCurrent + delta + length) % length;
  return Math.min(length - 1, Math.max(0, safeCurrent + delta));
}
