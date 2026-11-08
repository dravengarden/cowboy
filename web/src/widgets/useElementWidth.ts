import { type RefCallback, useCallback, useEffect, useRef, useState } from "react";

/** The width the element is actually given (docs/widgets.md §7.2): layout
 *  follows the container, never the surface name. Zero until measured. */
export function useElementWidth(): [RefCallback<HTMLElement>, number] {
  const [width, setWidth] = useState(0);
  const observer = useRef<ResizeObserver | null>(null);
  useEffect(() => () => observer.current?.disconnect(), []);
  const ref = useCallback((element: HTMLElement | null) => {
    observer.current?.disconnect();
    observer.current = null;
    if (!element || typeof ResizeObserver === "undefined") return;
    const next = new ResizeObserver((entries) => {
      const measured = Math.round(entries[0]?.contentRect.width ?? 0);
      setWidth((previous) => (previous === measured ? previous : measured));
    });
    next.observe(element);
    observer.current = next;
  }, []);
  return [ref, width];
}
