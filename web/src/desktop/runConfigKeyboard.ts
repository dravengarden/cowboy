export type RunConfigKeyAction =
  | { type: "field"; delta: -1 | 1 }
  | { type: "choice"; delta: -1 | 1 }
  | { type: "direct"; shortcut: "a" | "m" | "e" | "c" | "f" };

/** Resolve both standard arrows and visible Vim/mnemonic keys. */
export function runConfigKeyAction(key: string): RunConfigKeyAction | null {
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
    case "a":
    case "m":
    case "e":
    case "c":
    case "f":
      return {
        type: "direct",
        shortcut: key.toLowerCase() as "a" | "m" | "e" | "c" | "f",
      };
    default:
      return null;
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
