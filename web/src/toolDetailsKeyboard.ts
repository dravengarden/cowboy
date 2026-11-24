import { type MutableRefObject, type RefObject, useEffect } from "react";
import { isInteractiveControl, modalHandlerShouldSkip } from "./desktop/commands/modalGuard";
import { workspaceCommandKey } from "./desktop/commands/workspaceCommandKey";
import type { ToolItem, ToolRun } from "./tools/runs";

export function useToolDetailsKeyboard({
  desktop,
  item,
  runIndex,
  rawOnly,
  runs,
  dialogRootRef,
  navigationChord,
  setNavigationPrefixArmed,
  setRawByKey,
  select,
  onClose,
  onLocate,
}: {
  desktop: boolean;
  item: ToolItem | undefined;
  runIndex: number;
  rawOnly: boolean;
  runs: readonly ToolRun[];
  dialogRootRef: RefObject<HTMLDivElement | null>;
  navigationChord: MutableRefObject<number | null>;
  setNavigationPrefixArmed: (armed: boolean) => void;
  setRawByKey: React.Dispatch<React.SetStateAction<Record<string, boolean>>>;
  select: (next: ToolItem | undefined) => void;
  onClose: () => void;
  onLocate: (key: string) => void;
}): void {
  useEffect(() => {
    if (!desktop || !item) return undefined;
    const clearNavigationChord = (): void => {
      if (navigationChord.current !== null) {
        globalThis.clearTimeout(navigationChord.current);
        navigationChord.current = null;
      }
      setNavigationPrefixArmed(false);
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      if (modalHandlerShouldSkip(event, dialogRootRef.current)) return;
      if (event.metaKey || event.ctrlKey || event.altKey) {
        clearNavigationChord();
        return;
      }
      const key = workspaceCommandKey(event);
      if (navigationChord.current !== null) {
        event.preventDefault();
        event.stopPropagation();
        if (event.repeat) return;
        clearNavigationChord();
        if (key === "g") select(runs[0]?.tools[0]);
        return;
      }
      if (key === "Escape") {
        event.preventDefault();
        onClose();
      } else if (key === "[") {
        event.preventDefault();
        select(runs[runIndex - 1]?.tools.at(-1));
      } else if (key === "]") {
        event.preventDefault();
        select(runs[runIndex + 1]?.tools[0]);
      } else if (key === "j") {
        event.preventDefault();
        select(runs[Math.min(runs.length - 1, runIndex + 1)]?.tools[0]);
      } else if (key === "k") {
        event.preventDefault();
        select(runs[Math.max(0, runIndex - 1)]?.tools.at(-1));
      } else if (!rawOnly && (key === "h" || key === "l")) {
        event.preventDefault();
        setRawByKey((state) => ({ ...state, [item.key]: key === "l" }));
      } else if (key === "Enter") {
        if (isInteractiveControl(event.target)) return;
        event.preventDefault();
        onLocate(item.key);
      } else if (key === "g") {
        event.preventDefault();
        event.stopPropagation();
        if (event.repeat) return;
        setNavigationPrefixArmed(true);
        navigationChord.current = globalThis.setTimeout(() => {
          navigationChord.current = null;
          setNavigationPrefixArmed(false);
        }, 1200);
      } else if (key === "G") {
        event.preventDefault();
        select(runs.at(-1)?.tools.at(-1));
      }
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    return () => {
      globalThis.removeEventListener("keydown", onKeyDown, true);
      clearNavigationChord();
    };
  }, [desktop, runIndex, item, rawOnly, runs, onClose, onLocate]);
}
