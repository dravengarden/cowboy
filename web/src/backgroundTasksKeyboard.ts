import { type RefObject, useEffect, useLayoutEffect } from "react";
import { shortcutKey } from "./desktop/commands/imeShortcut";
import {
  isTopmostDialog,
  modalHandlerShouldSkip,
  owningDialog,
} from "./desktop/commands/modalGuard";

export interface BackgroundTasksKeyContext {
  readonly ownNode: HTMLElement | null;
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
  event: KeyboardEvent,
  ctx: BackgroundTasksKeyContext,
): void {
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  const dialog = owningDialog(ctx.ownNode);
  const targetInsideDialog =
    dialog !== null && event.target instanceof Node && dialog.contains(event.target);
  // When focus escaped behind the topmost modal (e.g. onto document.body after
  // a focused task/button unmounted, or onto the underlying Composer), evaluate
  // IME and editable guards against the non-editable modal itself so an idle
  // CJK source on a background editor cannot block inspector shortcuts.
  const guardEvent: KeyboardEvent = targetInsideDialog
    ? event
    : ({
      key: event.key,
      code: event.code,
      keyCode: event.keyCode,
      isComposing: event.isComposing,
      metaKey: event.metaKey,
      ctrlKey: event.ctrlKey,
      altKey: event.altKey,
      shiftKey: event.shiftKey,
      target: ctx.ownNode,
    } as unknown as KeyboardEvent);
  if (modalHandlerShouldSkip(guardEvent, ctx.ownNode)) return;
  if (
    targetInsideDialog &&
    event.target instanceof Element &&
    event.target.closest("[role='dialog'] [role='dialog']")
  ) return;
  const key = shortcutKey(guardEvent);
  if (!key) return;
  switch (key) {
    case "j":
    case "ArrowDown":
    case "]":
      event.preventDefault();
      event.stopPropagation();
      ctx.step(1);
      return;
    case "k":
    case "ArrowUp":
    case "[":
      event.preventDefault();
      event.stopPropagation();
      ctx.step(-1);
      return;
    case "o":
    case "O":
      if (ctx.selected?.source === "call") {
        event.preventDefault();
        event.stopPropagation();
        ctx.openConversation();
      }
      return;
    case "L":
      if (ctx.selected && ctx.selected.anchorSeq !== null) {
        event.preventDefault();
        event.stopPropagation();
        ctx.locate(ctx.selected.key);
      }
      return;
    case "x":
    case "X":
      if (ctx.selected?.canStop) {
        event.preventDefault();
        event.stopPropagation();
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
        event.stopPropagation();
        ctx.answerApproval();
      }
      return;
    case "h":
    case "Escape":
      event.preventDefault();
      event.stopPropagation();
      ctx.close();
      return;
  }
}

/** Own Desktop inspector shortcuts at window capture (matching ToolDetailsBrowser)
 *  so shortcuts keep working even if focus drops to document.body or a nested
 *  dialog closes, while keeping focus and selected-row visibility in sync. */
export function useBackgroundTasksKeyboard({
  enabled,
  selectedKey,
  inspectorRef,
  contextRef,
}: {
  enabled: boolean;
  selectedKey: string | null;
  inspectorRef: RefObject<HTMLDivElement | null>;
  contextRef: RefObject<BackgroundTasksKeyContext>;
}): void {
  useLayoutEffect(() => {
    if (!enabled) return undefined;
    const focusModal = (): void => {
      const node = inspectorRef.current;
      if (!node || !isTopmostDialog(node)) return;
      const dialog = owningDialog(node);
      if (
        dialog &&
        document.activeElement instanceof Node &&
        dialog.contains(document.activeElement)
      ) return;
      node.focus({ preventScroll: true });
    };
    focusModal();
    const frame = requestAnimationFrame(focusModal);
    return () => cancelAnimationFrame(frame);
  }, [enabled, inspectorRef]);

  useLayoutEffect(() => {
    if (!enabled || !selectedKey) return;
    const node = inspectorRef.current;
    if (!node) return;
    const option = node.querySelector<HTMLElement>(
      `[data-task-option="${CSS.escape(selectedKey)}"]`,
    );
    option?.scrollIntoView({ block: "nearest" });
    if (
      option &&
      document.activeElement instanceof HTMLElement &&
      document.activeElement !== option &&
      document.activeElement.hasAttribute("data-task-option") &&
      node.contains(document.activeElement)
    ) {
      option.focus({ preventScroll: true });
    }
  }, [enabled, selectedKey, inspectorRef]);

  useEffect(() => {
    if (!enabled) return undefined;
    const onKeyDown = (event: KeyboardEvent): void => {
      handleBackgroundTasksKeyDown(event, contextRef.current);
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    return () => globalThis.removeEventListener("keydown", onKeyDown, true);
  }, [enabled, contextRef]);
}
