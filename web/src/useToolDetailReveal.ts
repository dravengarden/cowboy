import { type RefObject, useEffect, useLayoutEffect, useRef, useState } from "react";

// Opening waits out the sheet's slide so a heavy body cannot drop its frames.
// Moving inside an open inspector has no slide to protect: paint the new
// heading, then mount its body on the next frames.
const OPEN_RENDER_DELAY_MS = 320;
const NAVIGATION_RENDER_DELAY_MS = 32;
const OPEN_SETTLE_MS = 340;
const NAVIGATION_SETTLE_MS = 96;

export function scrollableAncestor(node: HTMLElement | null): HTMLElement | null {
  let current = node?.parentElement ?? null;
  while (current) {
    const overflow = globalThis.getComputedStyle(current).overflowY;
    if (overflow === "auto" || overflow === "scroll") return current;
    current = current.parentElement;
  }
  return null;
}

export function useToolDetailReveal({
  itemKey,
  bodyRef,
  currentRef,
  anchorSpacerRef,
  scrollByKey,
}: {
  itemKey: string | undefined;
  bodyRef: RefObject<HTMLElement | null>;
  currentRef: RefObject<HTMLElement | null>;
  anchorSpacerRef: RefObject<HTMLElement | null>;
  scrollByKey: RefObject<Map<string, number>>;
}): { detailReadyKey: string | null; markDetailReady: (key: string) => void } {
  const [detailReadyKey, setDetailReadyKey] = useState<string | null>(null);
  const opened = useRef(false);

  useEffect(() => {
    if (itemKey === undefined) {
      opened.current = false;
      return undefined;
    }
    const id = globalThis.setTimeout(
      () => {
        opened.current = true;
        setDetailReadyKey(itemKey);
      },
      opened.current ? NAVIGATION_RENDER_DELAY_MS : OPEN_RENDER_DELAY_MS,
    );
    return () => globalThis.clearTimeout(id);
  }, [itemKey]);

  useLayoutEffect(() => {
    if (itemKey === undefined) return;
    const scroller = scrollableAncestor(bodyRef.current);
    const current = currentRef.current;
    const spacer = anchorSpacerRef.current;
    if (!scroller || !current || !spacer) return;
    const saved = scrollByKey.current.get(itemKey);
    const align = (): void => {
      // Keep BOTH context directions in normal scroll flow, but add only the
      // trailing room needed for the selected tool to reach the top on a short
      // page. This is layout compensation, not visible content: opening focuses
      // the tool; scrolling upward reveals its preceding prose, downward reveals
      // its following prose. Re-run after the sheet computes its cover height.
      spacer.style.height = "0px";
      const currentTop =
        scroller.scrollTop +
        current.getBoundingClientRect().top -
        scroller.getBoundingClientRect().top;
      // A bleeding reader scrolls under the status area; stop below that inset.
      const inset = Number.parseFloat(globalThis.getComputedStyle(scroller).scrollPaddingTop) || 0;
      const target = Math.max(0, currentTop - inset);
      const maxWithoutSpacer = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      spacer.style.height = `${Math.max(0, target - maxWithoutSpacer + 8)}px`;
      scroller.scrollTop = saved ?? target;
    };
    align();
    const frame = globalThis.requestAnimationFrame(align);
    const settled = globalThis.setTimeout(
      align,
      opened.current ? NAVIGATION_SETTLE_MS : OPEN_SETTLE_MS,
    );
    return () => {
      globalThis.cancelAnimationFrame(frame);
      globalThis.clearTimeout(settled);
    };
  }, [itemKey]);

  return { detailReadyKey, markDetailReady: setDetailReadyKey };
}
