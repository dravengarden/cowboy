import { useLayoutEffect, useRef } from "react";

/** Attach to the native scroll owner, without adding a wrapper or changing layout.
 * Place last in a Stack/List so its invisible marker cannot affect item spacing.
 * Masks reveal the actual surface beneath, so every theme uses its own colour.
 */
export function ScrollEdges({ top = true, bottom = true }: { top?: boolean; bottom?: boolean }) {
  const marker = useRef<HTMLTemplateElement>(null);
  useLayoutEffect(() => {
    const owner = marker.current?.parentElement;
    if (!owner) return;
    const previous = { mask: owner.style.maskImage, webkit: owner.style.webkitMaskImage };
    let frame = 0, last = "";
    const update = () => {
      frame = 0;
      const overflow = getComputedStyle(owner).overflowY;
      const focused = document.activeElement;
      const editable = owner.contains(focused) && focused?.matches("input, textarea, [contenteditable=true], .cm-content, :focus-visible");
      // A mask also clips fixed descendants. Inline pickers must escape intact.
      const overlay = owner.querySelector('[role="dialog"], [role="menu"]');
      const remaining = owner.scrollHeight - owner.clientHeight - owner.scrollTop;
      const scrolling = (overflow === "auto" || overflow === "scroll") && owner.scrollHeight > owner.clientHeight + 1 && !editable && !overlay;
      const sticky = top && [...owner.querySelectorAll("[data-fold-toolbar]")].some(node => getComputedStyle(node).position === "sticky");
      const leading = scrolling && top && owner.scrollTop > 1 && !sticky;
      const trailing = scrolling && bottom && remaining > 1;
      const value = leading || trailing
        ? `linear-gradient(to bottom, ${leading ? "transparent, black 16px" : "black 0px"}, ${trailing ? "black calc(100% - 16px), transparent" : "black 100%"})`
        : "none";
      if (value === last) return;
      last = value;
      owner.style.maskImage = value;
      owner.style.webkitMaskImage = value;
      owner.dataset.scrollEdges = `${leading ? "top" : ""} ${trailing ? "bottom" : ""}`.trim();
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };
    const resize = new ResizeObserver(schedule);
    const observe = () => {
      resize.disconnect();
      resize.observe(owner);
      for (const child of owner.children) if (child !== marker.current) resize.observe(child);
      schedule();
    };
    const mutation = new MutationObserver(records => {
      if (records.some(record => record.type === "childList" && record.target === owner)) observe();
      else schedule();
    });
    mutation.observe(owner, { childList: true, characterData: true, subtree: true });
    owner.addEventListener("scroll", schedule, { passive: true });
    owner.addEventListener("focusin", schedule);
    owner.addEventListener("focusout", schedule);
    observe();
    return () => {
      cancelAnimationFrame(frame);
      mutation.disconnect(); resize.disconnect();
      owner.removeEventListener("scroll", schedule);
      owner.removeEventListener("focusin", schedule);
      owner.removeEventListener("focusout", schedule);
      owner.style.maskImage = previous.mask; owner.style.webkitMaskImage = previous.webkit;
      delete owner.dataset.scrollEdges;
    };
  }, [top, bottom]);
  return <template ref={marker} />;
}
