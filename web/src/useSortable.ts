import { useCallback, useEffect, useRef, useState } from "react";
import type {
  CSSProperties,
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";

// A small, dependency-free vertical drag-to-reorder hook. Reorder is driven from
// a dedicated GRIP HANDLE per row (not the whole row), so it never fights the
// list's scroll or the DetentSheet's drag-to-dismiss: the handle's pointerdown
// stops propagation and claims the gesture, while tapping the rest of the row
// still does its normal thing. The dragged row tracks the finger (transform);
// the other rows slide to open a gap (CSS transition). Order is the caller's —
// on drop we hand back the new id order and the server echoes it (so all
// terminals stay in sync); an optimistic local order bridges the round-trip so
// the row never snaps back. Built for short lists (sessions / queue / drafts),
// so there's no virtualization or edge auto-scroll.

interface DragState {
  id: string;
  startY: number;
  dy: number;
  originIndex: number;
  targetIndex: number;
  /** Center-to-center row spacing (px) — the gap a shifted row opens. */
  step: number;
}

export interface Sortable {
  /** The id order to render rows in (optimistic during/just-after a drag). */
  order: string[];
  draggingId: string | null;
  /** Ref callback to register a row's DOM node (for measuring spacing). */
  registerItem: (id: string) => (el: HTMLElement | null) => void;
  /** Style for a row container (the drag transform / gap shift). */
  itemStyle: (id: string) => CSSProperties;
  /** Props to spread on the grip handle element. */
  handleProps: (id: string) => {
    onPointerDown: (e: ReactPointerEvent) => void;
    onClick: (e: ReactMouseEvent) => void;
    style: CSSProperties;
  };
}

export function useSortable(opts: {
  ids: string[];
  onReorder: (newIds: string[]) => void;
  onDragStart?: (() => void) | undefined;
  onDragEnd?: (() => void) | undefined;
}): Sortable {
  const { ids, onReorder, onDragStart, onDragEnd } = opts;
  const nodes = useRef(new Map<string, HTMLElement>());
  const [drag, setDrag] = useState<DragState | null>(null);
  // Bridge the drop → server-echo round-trip so rows don't snap back to the old
  // order for a frame. Cleared whenever a fresh `ids` arrives (the server spoke).
  const [optimistic, setOptimistic] = useState<string[] | null>(null);

  const dragRef = useRef<DragState | null>(null);
  dragRef.current = drag;
  const idsRef = useRef(ids);
  idsRef.current = ids;
  const cbRef = useRef({ onReorder, onDragStart, onDragEnd });
  cbRef.current = { onReorder, onDragStart, onDragEnd };

  useEffect(() => {
    setOptimistic(null);
  }, [ids]);

  const registerItem = useCallback(
    (id: string) => (el: HTMLElement | null) => {
      if (el) nodes.current.set(id, el);
      else nodes.current.delete(id);
    },
    [],
  );

  // Window listeners live only while a drag is active (re-bound on start/end).
  useEffect(() => {
    if (!drag) return undefined;
    const move = (e: PointerEvent): void => {
      const d = dragRef.current;
      if (!d) return;
      const dy = e.clientY - d.startY;
      const target = Math.max(
        0,
        Math.min(idsRef.current.length - 1, d.originIndex + Math.round(dy / d.step)),
      );
      if (dy !== d.dy || target !== d.targetIndex) {
        setDrag({ ...d, dy, targetIndex: target });
      }
    };
    const up = (): void => {
      const d = dragRef.current;
      if (d && d.targetIndex !== d.originIndex) {
        const next = [...idsRef.current];
        const [moved] = next.splice(d.originIndex, 1);
        if (moved !== undefined) {
          next.splice(d.targetIndex, 0, moved);
          setOptimistic(next);
          cbRef.current.onReorder(next);
        }
      }
      setDrag(null);
      cbRef.current.onDragEnd?.();
    };
    globalThis.addEventListener("pointermove", move);
    globalThis.addEventListener("pointerup", up);
    globalThis.addEventListener("pointercancel", up);
    return () => {
      globalThis.removeEventListener("pointermove", move);
      globalThis.removeEventListener("pointerup", up);
      globalThis.removeEventListener("pointercancel", up);
    };
  }, [drag === null]);

  const handleProps = useCallback(
    (id: string) => ({
      onPointerDown: (e: ReactPointerEvent): void => {
        if (e.button !== 0 && e.button !== -1) return; // left / touch only
        // Claim the gesture: no row tap, no list scroll, no sheet drag.
        e.preventDefault();
        e.stopPropagation();
        const index = idsRef.current.indexOf(id);
        if (index < 0) return;
        const el = nodes.current.get(id);
        const neighborId = idsRef.current[index + 1] ?? idsRef.current[index - 1];
        const nb = neighborId ? nodes.current.get(neighborId) : undefined;
        let step = el?.offsetHeight ?? 48;
        if (el && nb) {
          step = Math.abs(nb.getBoundingClientRect().top - el.getBoundingClientRect().top) || step;
        }
        setDrag({ id, startY: e.clientY, dy: 0, originIndex: index, targetIndex: index, step });
        cbRef.current.onDragStart?.();
      },
      onClick: (e: ReactMouseEvent): void => e.stopPropagation(),
      style: { touchAction: "none", cursor: "grab" } as CSSProperties,
    }),
    [],
  );

  const itemStyle = useCallback(
    (id: string): CSSProperties => {
      if (!drag) return {};
      if (id === drag.id) {
        return {
          transform: `translateY(${String(drag.dy)}px)`,
          zIndex: 5,
          position: "relative",
          transition: "none",
          opacity: 0.92,
        };
      }
      const idx = idsRef.current.indexOf(id);
      let shift = 0;
      if (drag.targetIndex > drag.originIndex && idx > drag.originIndex && idx <= drag.targetIndex) {
        shift = -drag.step;
      } else if (
        drag.targetIndex < drag.originIndex &&
        idx >= drag.targetIndex &&
        idx < drag.originIndex
      ) {
        shift = drag.step;
      }
      return {
        transform: `translateY(${String(shift)}px)`,
        transition: "transform 0.18s ease",
        position: "relative",
      };
    },
    [drag],
  );

  // During a drag, render in the stable server order (transforms show the
  // rearrangement); after drop, the optimistic order until the echo lands.
  const order = drag ? ids : (optimistic ?? ids);
  return { order, draggingId: drag?.id ?? null, registerItem, itemStyle, handleProps };
}
