import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { shortcutKey } from "./desktop/commands/imeShortcut";
import { modalHandlerShouldSkip } from "./desktop/commands/modalGuard";

export interface BackgroundTasksKeyContext {
  readonly ownNode: HTMLElement | null;
  readonly typing: boolean;
  readonly selected: {
    readonly key: string;
    readonly source: string;
    readonly anchorSeq: number | null;
    readonly canStop: boolean;
  } | null;
  readonly step: (delta: number) => void;
  readonly openConversation: () => void;
  readonly locate: (key: string) => void;
  readonly answerApproval: () => void;
  readonly close: () => void;
}

export function handleBackgroundTasksKeyDown(
  event: ReactKeyboardEvent<HTMLElement>,
  ctx: BackgroundTasksKeyContext,
): void {
  if (event.metaKey || event.ctrlKey || event.altKey || ctx.typing) return;
  if (modalHandlerShouldSkip(event.nativeEvent, ctx.ownNode)) return;
  if ((event.target as HTMLElement).closest?.("[role='dialog'] [role='dialog']")) return;
  switch (shortcutKey(event.nativeEvent)) {
    case "j":
    case "ArrowDown":
    case "]":
      event.preventDefault();
      ctx.step(1);
      return;
    case "k":
    case "ArrowUp":
    case "[":
      event.preventDefault();
      ctx.step(-1);
      return;
    case "o":
      if (ctx.selected?.source === "call") {
        event.preventDefault();
        ctx.openConversation();
      }
      return;
    case "L":
      if (ctx.selected && ctx.selected.anchorSeq !== null) {
        event.preventDefault();
        ctx.locate(ctx.selected.key);
      }
      return;
    case "x":
    case "X":
      if (ctx.selected?.canStop) {
        event.preventDefault();
        const detail = document.querySelector<HTMLElement>(
          `[data-task-detail="${CSS.escape(ctx.selected.key)}"]`,
        );
        (detail?.querySelector<HTMLButtonElement>("[data-task-detail-stop]") ??
          [...(detail?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
            .find((button) => /^stop call$/i.test(button.textContent?.trim() ?? "")))?.click();
      }
      return;
    case "Enter":
      if (ctx.selected?.source === "approval") {
        event.preventDefault();
        ctx.answerApproval();
      }
      return;
    case "h":
    case "Escape":
      event.preventDefault();
      ctx.close();
      return;
  }
}
