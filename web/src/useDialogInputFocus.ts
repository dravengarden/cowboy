import { type RefObject, useLayoutEffect, useRef } from "react";

/**
 * Focus an element of a freshly mounted dialog.
 *
 * The synchronous attempt keeps iOS's contract: the opener mounts the dialog
 * with flushSync inside the tap, so focusing here still owns user activation
 * and raises the software keyboard. Desktop dialogs mount their portal one
 * commit later and MUI's focus trap then parks focus on the paper, so with
 * `retry` the element is claimed again once that has settled; otherwise a
 * keyboard-opened prompt would swallow the first keystrokes.
 */
export function useDialogFocus(
  find: () => HTMLElement | null | undefined,
  retry: boolean,
): void {
  const findRef = useRef(find);
  findRef.current = find;
  useLayoutEffect(() => {
    const claim = (): void => {
      const target = findRef.current();
      if (!target || document.activeElement === target) return;
      target.focus({ preventScroll: true });
      if (target instanceof HTMLInputElement) target.select();
    };
    claim();
    if (!retry) return undefined;
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(claim);
    });
    const timer = globalThis.setTimeout(claim, 120);
    return () => {
      cancelAnimationFrame(frame);
      globalThis.clearTimeout(timer);
    };
  }, []);
}

/** {@link useDialogFocus} for a prompt's text field (focus and select). */
export function useDialogInputFocus(
  inputRef: RefObject<HTMLInputElement | null>,
  retry: boolean,
): void {
  useDialogFocus(() => inputRef.current, retry);
}
