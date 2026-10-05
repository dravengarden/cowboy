import { useCallback, useEffect, useRef, useState } from "react";
import type {
  CSSProperties,
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import { haptic } from "./haptic";
import { sortableTargetIndex } from "./sortableGeometry";

/** Touch lifts a row after this still hold: long enough to reject a finger
 *  brushing the grip while it scrolls or swipes, short enough not to feel like
 *  waiting (iOS's own list reorder lifts in about the same time). */
export const TOUCH_HOLD_MS = 180;
/** Movement allowed during the hold before it counts as a scroll instead. */
export const TOUCH_HOLD_SLOP_PX = 8;

// A small, dependency-free vertical drag-to-reorder hook. Reorder is driven from
// a dedicated GRIP HANDLE per row (not the whole row), so it never fights the
// list's scroll or the DetentSheet's drag-to-dismiss: the handle's pointerdown
// stops propagation and claims the gesture, while tapping the rest of the row
// still does its normal thing. The dragged row tracks the finger (transform);
// the other rows slide to open a gap (CSS transition). Order is the caller's —
// on drop we hand back the new id order and the server echoes it (so all
// terminals stay in sync); an optimistic local order bridges the round-trip so
// the row never snaps back. Built for short lists (sessions / queue / drafts):
// no virtualization, but it DOES edge auto-scroll — a finger held near the
// scrolling edge keeps the list moving, and the drop target is tracked in
// content space so it stays correct as the list moves under a still finger. The
// element that actually scrolls is RESOLVED at pickup (see `resolveScroller`),
// not assumed, so the same hook works in a bounded column AND a content-height
// sheet where a parent does the scrolling.

// Resolve the element that actually SCROLLS for edge auto-scroll. The caller's
// `scrollContainer` hint is honored verbatim when it really scrolls (desktop
// sidebar: the <List> IS the overflow:auto column). But in a content-height
// sheet (the mobile DetentSheet that hosts the session list, or a composer
// drafts/queue panel) the hinted node is sized to its content — the SHEET BODY
// scrolls, not the inner list — so the hint's scrollHeight == clientHeight and
// edge-scroll would silently no-op (the bug: rows reorder but never auto-scroll
// on mobile). Fall back to walking up from the dragged row to the nearest
// scrollable ancestor (dnd-kit's getScrollableAncestors approach) so every
// layout works without the caller having to know which element scrolls.
const SCROLLABLE_OVERFLOW = new Set(["auto", "scroll", "overlay"]);
function actuallyScrolls(el: HTMLElement): boolean {
  return el.scrollHeight > el.clientHeight + 1;
}
function resolveScroller(
  node: HTMLElement | null,
  hint: HTMLElement | null,
): HTMLElement | null {
  if (hint && actuallyScrolls(hint)) return hint;
  const doc = globalThis.document;
  let el: HTMLElement | null = node;
  while (el && el !== doc.body && el !== doc.documentElement) {
    if (
      SCROLLABLE_OVERFLOW.has(globalThis.getComputedStyle(el).overflowY) &&
      actuallyScrolls(el)
    ) {
      return el;
    }
    el = el.parentElement;
  }
  return hint; // nothing inner scrolls (yet) — keep the caller's intent
}

interface DragState {
  id: string;
  startY: number;
  startX: number;
  originIndex: number;
  targetIndex: number;
  /** Horizontal intent in whole `horizontalStep`s (0 when disabled). */
  depthSteps: number;
  overId?: string | null;
  /** Viewport tops and heights of every row at pickup, in `ids` order. Rows
   *  may differ in height (a folder header is shorter than a session row), so
   *  the drop slot is decided against each row's own midpoint rather than a
   *  uniform step that drifts on mixed lists. */
  tops: number[];
  heights: number[];
  /** Space the dragged row occupies (its height plus the gap to its
   *  neighbour) — the gap a shifted row opens. */
  slot: number;
  /** Container scrollTop at pickup. The live (clientY − startY) is measured in
   *  viewport space; adding (scrollTop − startScrollTop) converts it to CONTENT
   *  space, so the row + drop target stay right when the list auto-scrolls. */
  startScrollTop: number;
}

/** What a caller needs to project an in-flight drag (folder trees). */
export interface SortableDrag {
  id: string;
  originIndex: number;
  targetIndex: number;
  depthSteps: number;
  overId?: string | null;
}

export interface Sortable {
  /** The id order to render rows in (optimistic during/just-after a drag). */
  order: string[];
  draggingId: string | null;
  /** The in-flight drag, re-published only when its slot or depth changes. */
  drag: SortableDrag | null;
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
  /** Called when a drop changed its slot or horizontal intent. A grip tap
   *  is not a move. When present it replaces `onReorder`. */
  onDrop?: ((newIds: string[], drag: SortableDrag) => void) | undefined;
  /** Opt-in center drops, measured from cached pickup geometry. */
  itemDrop?: ((id: string) => boolean) | undefined;
  onDragStart?: (() => void) | undefined;
  onDragEnd?: (() => void) | undefined;
  /** Hint at the scrollable container for edge auto-scroll, or null. A getter
   *  (not a ref) so the caller's element type never has to match the hook's.
   *  Used verbatim only when it actually scrolls; otherwise the hook walks up
   *  from the dragged row to the real scroll parent (see `resolveScroller`). */
  scrollContainer?: (() => HTMLElement | null) | undefined;
  /** Pointer distance (px) per unit of horizontal intent; off when absent. */
  horizontalStep?: number | undefined;
  /** Extra X offset for the dragged row (the caller's projected indent). */
  dragOffsetX?: number | undefined;
  /** Tree owners already project optimistic placement/order together. A flat
   * post-drop permutation would temporarily put root rows inside a branch. */
  optimisticReorder?: boolean | undefined;
}): Sortable {
  const {
    ids,
    onReorder,
    onDrop,
    itemDrop,
    onDragStart,
    onDragEnd,
    scrollContainer,
    horizontalStep,
    dragOffsetX = 0,
    optimisticReorder = true,
  } = opts;
  const nodes = useRef(new Map<string, HTMLElement>());
  const [drag, setDrag] = useState<DragState | null>(null);
  // Bridge the drop → server-echo round-trip so rows don't snap back to the old
  // order for a frame. Cleared whenever a fresh `ids` arrives (the server spoke).
  const [optimistic, setOptimistic] = useState<string[] | null>(null);

  const dragRef = useRef<DragState | null>(null);
  dragRef.current = drag;
  // The dragged row's continuous finger offset. Kept in a REF, not state, so a
  // pointermove that doesn't cross a row boundary writes the transform straight
  // to the DOM node (below) without a React re-render — the difference between a
  // smooth 60fps drag and re-rendering the whole list every pixel. Only the
  // discrete `targetIndex` (which flips ~once per row crossed) goes through
  // state, to drive the other rows' gap shift.
  const dyRef = useRef(0);
  const offsetXRef = useRef(dragOffsetX);
  offsetXRef.current = dragOffsetX;
  // Last pointer position, cached on every pointermove. The auto-scroll rAF
  // loop reads it each frame so it keeps working while the finger is held
  // STILL at an edge (pointermove stops firing then).
  const lastYRef = useRef(0);
  const lastXRef = useRef(0);
  const idsRef = useRef(ids);
  idsRef.current = ids;
  const stepRef = useRef(horizontalStep);
  stepRef.current = horizontalStep;
  // Hold the scrollContainer getter in a ref so the drag effect (re-bound only on
  // start/stop) always calls the latest one.
  const scRef = useRef(scrollContainer);
  scRef.current = scrollContainer;
  // The element resolved to actually scroll, fixed for the duration of one drag
  // (resolved at pickup from the dragged row + the caller's hint). apply()/tick()
  // read this rather than re-resolving every frame.
  const scrollElRef = useRef<HTMLElement | null>(null);
  const cbRef = useRef({
    onReorder,
    onDrop,
    itemDrop,
    onDragStart,
    onDragEnd,
    optimisticReorder,
  });
  cbRef.current = {
    onReorder,
    onDrop,
    itemDrop,
    onDragStart,
    onDragEnd,
    optimisticReorder,
  };

  // Clear the optimistic order once the ids CONTENT changes (a server echo, or a
  // draft added/removed). Keyed on the joined ids, NOT the array ref: callers
  // rebuild `ids` every render, so `[ids]` would fire every render — clearing
  // optimistic after one frame (snap-back) and racing a stale value on screen.
  const idsKey = ids.join(" ");
  useEffect(() => {
    setOptimistic(null);
  }, [idsKey]);

  const registerItem = useCallback(
    (id: string) => (el: HTMLElement | null) => {
      if (el) nodes.current.set(id, el);
      else nodes.current.delete(id);
    },
    [],
  );

  // The caller's projected indent can change on a re-render that the pointer
  // did not cause (its projection follows the new slot); restate it.
  useEffect(() => {
    const d = dragRef.current;
    const node = d ? nodes.current.get(d.id) : undefined;
    if (node) {
      node.style.transform = `translate3d(${String(dragOffsetX)}px, ${
        String(dyRef.current)
      }px, 0)`;
    }
  }, [dragOffsetX]);

  // Window listeners + the auto-scroll loop live only while a drag is active
  // (re-bound on start/end).
  useEffect(() => {
    if (!drag) return undefined;

    // Re-place the dragged row + recompute the drop target from the CACHED pointer
    // and the container's CURRENT scrollTop. Working in content space (adding the
    // scroll delta since pickup) keeps the row glued to the finger and the target
    // correct whether the finger moved OR the list auto-scrolled under a still
    // finger. Shared by the pointermove handler and the rAF scroll loop.
    const apply = (): void => {
      const d = dragRef.current;
      if (!d) return;
      const sc = scrollElRef.current;
      const scrollDelta = sc ? sc.scrollTop - d.startScrollTop : 0;
      const effDy = lastYRef.current - d.startY + scrollDelta;
      // Drive the dragged row's transform imperatively — no setState per pixel.
      dyRef.current = effDy;
      const node = nodes.current.get(d.id);
      if (node) {
        node.style.transform = `translate3d(${String(offsetXRef.current)}px, ${
          String(effDy)
        }px, 0)`;
      }
      const target = sortableTargetIndex(
        d.tops,
        d.heights,
        d.originIndex,
        effDy,
      );
      const step = stepRef.current;
      const depthSteps = step
        ? Math.round((lastXRef.current - d.startX) / step)
        : 0;
      let overId: string | null = null;
      if (cbRef.current.itemDrop?.(d.id)) {
        const y = lastYRef.current + scrollDelta;
        const index = d.tops.findIndex((top, index) => index !== d.originIndex &&
          y >= top + d.heights[index]! * 0.3 && y <= top + d.heights[index]! * 0.7);
        overId = index >= 0 ? idsRef.current[index] ?? null : null;
      }
      // Only a slot or depth change re-renders (to slide the other rows' gap).
      if (target !== d.targetIndex || depthSteps !== d.depthSteps || overId !== (d.overId ?? null)) {
        const next = { ...d, targetIndex: target, depthSteps, overId };
        // pointerup can arrive before React commits the last pointermove.
        dragRef.current = next;
        setDrag(next);
      }
    };

    const move = (e: PointerEvent): void => {
      if (!dragRef.current) return;
      lastYRef.current = e.clientY; // pointer events only cache the position now.
      lastXRef.current = e.clientX;
      apply();
    };

    // Edge auto-scroll. A rAF loop (NOT pointermove-driven): a finger parked in the
    // edge band must keep scrolling. Velocity ramps quadratically with how deep the
    // finger is in the band, dt-scaled so 60/120Hz feel identical. A light haptic
    // ticks on ENGAGE and once at the BOUNDARY (top/bottom reached while still
    // pulling), gated by a tiny state machine so it never buzzes per frame.
    const MAX_V = 1000; // px/s at the very edge
    let raf = 0;
    let prevTs = 0;
    let hap: "idle" | "scrolling" | "boundary" = "idle";
    const tick = (ts: number): void => {
      raf = globalThis.requestAnimationFrame(tick);
      const sc = scrollElRef.current;
      if (prevTs === 0) prevTs = ts;
      const dt = Math.min(0.05, (ts - prevTs) / 1000); // clamp first/stutter frames
      prevTs = ts;
      if (!sc || !dragRef.current || sc.scrollHeight <= sc.clientHeight + 1) {
        hap = "idle";
        return;
      }
      const rect = sc.getBoundingClientRect();
      const zone = Math.min(64, rect.height * 0.18);
      const y = lastYRef.current;
      const topGap = y - rect.top;
      const botGap = rect.bottom - y;
      const canUp = sc.scrollTop > 0;
      const canDown = sc.scrollTop < sc.scrollHeight - sc.clientHeight - 1;
      let dir = 0;
      let depth = 0;
      let blocked = false;
      if (topGap < zone) {
        if (canUp) {
          dir = -1;
          depth = (zone - topGap) / zone;
        } else blocked = true;
      } else if (botGap < zone) {
        if (canDown) {
          dir = 1;
          depth = (zone - botGap) / zone;
        } else blocked = true;
      }
      if (dir !== 0) {
        if (hap === "idle") {
          hap = "scrolling";
          haptic(8); // light tick — auto-scroll engaged
        }
        const t = Math.min(1, Math.max(0, depth));
        sc.scrollTop += dir * MAX_V * t * t * dt;
        apply(); // the list moved under the finger → re-place row + retarget
      } else if (blocked) {
        // Pulling past an edge we can't cross: one thunk, and ONLY if we were
        // already scrolling — so hovering a maxed-out list on entry stays silent.
        if (hap === "scrolling") {
          hap = "boundary";
          haptic(8);
        }
      } else {
        hap = "idle"; // out of both bands → re-arm
      }
    };
    raf = globalThis.requestAnimationFrame(tick);

    const finish = (commit: boolean): void => {
      const d = dragRef.current;
      if (d && commit) {
        const next = [...idsRef.current];
        if (d.targetIndex !== d.originIndex) {
          const [moved] = next.splice(d.originIndex, 1);
          if (moved !== undefined) next.splice(d.targetIndex, 0, moved);
        }
        const moved = d.targetIndex !== d.originIndex;
        if (moved && cbRef.current.optimisticReorder) setOptimistic(next);
        const { onDrop: drop, onReorder: reorder } = cbRef.current;
        if (drop && (moved || d.depthSteps !== 0 || d.overId != null)) {
          drop(next, {
            id: d.id,
            originIndex: d.originIndex,
            targetIndex: d.targetIndex,
            depthSteps: d.depthSteps,
            overId: d.overId ?? null,
          });
        } else if (!drop && moved) {
          reorder(next);
        }
      }
      const node = d ? nodes.current.get(d.id) : undefined;
      if (node) node.style.transform = "";
      dragRef.current = null;
      setDrag(null);
      scrollElRef.current = null; // don't pin a node past the drag
      if (commit) haptic();
      cbRef.current.onDragEnd?.();
    };
    const up = (event: PointerEvent): void => {
      lastYRef.current = event.clientY;
      lastXRef.current = event.clientX;
      apply();
      finish(true);
    };
    const cancel = (): void => finish(false);
    globalThis.addEventListener("pointermove", move);
    globalThis.addEventListener("pointerup", up);
    globalThis.addEventListener("pointercancel", cancel);
    return () => {
      globalThis.cancelAnimationFrame(raf);
      globalThis.removeEventListener("pointermove", move);
      globalThis.removeEventListener("pointerup", up);
      globalThis.removeEventListener("pointercancel", cancel);
    };
  }, [drag === null]);

  // A pending touch hold (see handleProps); cancelled on unmount.
  const holdRef = useRef<(() => void) | null>(null);
  useEffect(() => () => holdRef.current?.(), []);
  // Rows lift through refs and the state setter only, so this stays stable.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const lift = useCallback((id: string, x: number, y: number): void => liftRow(id, x, y), []);
  const handleProps = useCallback(
    (id: string) => ({
      onPointerDown: (e: ReactPointerEvent): void => {
        if (e.button !== 0 && e.button !== -1) return; // left / touch only
        // Claim the gesture: no row tap, no list scroll, no sheet drag.
        e.preventDefault();
        e.stopPropagation();
        if (e.pointerType === "mouse") {
          lift(id, e.clientX, e.clientY);
          return;
        }
        // Touch and pen lift only after a short, still hold, so a finger
        // that merely brushes the grip while scrolling or swiping the drawer
        // never reorders. Moving away or releasing first cancels.
        holdRef.current?.();
        const pointerId = e.pointerId;
        const startX = e.clientX;
        const startY = e.clientY;
        let x = startX;
        let y = startY;
        const cancel = (): void => {
          globalThis.clearTimeout(timer);
          globalThis.removeEventListener("pointermove", onMove);
          globalThis.removeEventListener("pointerup", onEnd);
          globalThis.removeEventListener("pointercancel", onEnd);
          if (holdRef.current === cancel) holdRef.current = null;
        };
        const onMove = (event: PointerEvent): void => {
          if (event.pointerId !== pointerId) return;
          x = event.clientX;
          y = event.clientY;
          if (Math.hypot(x - startX, y - startY) > TOUCH_HOLD_SLOP_PX) cancel();
        };
        const onEnd = (event: PointerEvent): void => {
          if (event.pointerId === pointerId) cancel();
        };
        const timer = globalThis.setTimeout(() => {
          cancel();
          lift(id, x, y);
        }, TOUCH_HOLD_MS);
        holdRef.current = cancel;
        globalThis.addEventListener("pointermove", onMove);
        globalThis.addEventListener("pointerup", onEnd);
        globalThis.addEventListener("pointercancel", onEnd);
      },
      onClick: (e: ReactMouseEvent): void => e.stopPropagation(),
      style: {
        touchAction: "none",
        cursor: "grab",
        userSelect: "none",
        WebkitUserSelect: "none",
        WebkitTouchCallout: "none",
      } as CSSProperties,
    }),
    [lift],
  );

  // Pick a row up at (clientX, clientY): measure the list once and start the
  // drag. Mouse lifts on press; touch after the hold above.
  function liftRow(id: string, clientX: number, clientY: number): void {
    const index = idsRef.current.indexOf(id);
    if (index < 0) return;
    // Measure every row once, at pickup. Rows do not change size during
    // a drag (they only translate), so this geometry stays valid.
    const tops: number[] = [];
    const heights: number[] = [];
    for (const rowId of idsRef.current) {
      const rect = nodes.current.get(rowId)?.getBoundingClientRect();
      const previousTop = tops.at(-1);
      const previousHeight = heights.at(-1);
      const top = rect?.top ??
        (previousTop !== undefined && previousHeight !== undefined
          ? previousTop + previousHeight
          : 0);
      tops.push(top);
      heights.push(rect?.height ?? 48);
    }
    const height = heights[index] ?? 48;
    const nextTop = tops[index + 1];
    const previousTop = tops[index - 1];
    const previousHeight = heights[index - 1];
    // The dragged row's slot: its height plus the gap that follows it (or,
    // for the last row, the gap that precedes it).
    const gap = nextTop !== undefined
      ? nextTop - (tops[index] ?? 0) - height
      : previousTop !== undefined && previousHeight !== undefined
      ? (tops[index] ?? 0) - previousTop - previousHeight
      : 0;
    dyRef.current = 0;
    lastYRef.current = clientY;
    lastXRef.current = clientX;
    // Pin the real scroll element for this drag: the caller's hint if it
    // scrolls, else the nearest scrollable ancestor of the dragged row (so a
    // content-height sheet, where a parent scrolls, still edge-scrolls).
    const el = nodes.current.get(id);
    const sc = resolveScroller(el ?? null, scRef.current?.() ?? null);
    scrollElRef.current = sc;
    const startScrollTop = sc?.scrollTop ?? 0;
    setDrag({
      id,
      startY: clientY,
      startX: clientX,
      originIndex: index,
      targetIndex: index,
      depthSteps: 0,
      tops,
      heights,
      slot: height + Math.max(0, gap),
      startScrollTop,
    });
    haptic(24); // firmer "lift" on PICKUP (iOS reorder feel), before onDragStart
    cbRef.current.onDragStart?.();
  }

  const itemStyle = useCallback(
    (id: string): CSSProperties => {
      if (!drag) return {};
      if (id === drag.id) {
        // `dyRef` (a ref) carries the live offset; the pointermove handler also
        // writes this transform straight to the node, so a re-render here (only
        // on a boundary cross) just re-states the same value — no fight.
        // `willChange` promotes the row to its own compositor layer for a
        // jank-free GPU transform.
        return {
          transform: `translate3d(${String(dragOffsetX)}px, ${
            String(dyRef.current)
          }px, 0)`,
          zIndex: 5,
          position: "relative",
          transition: "none",
          opacity: 0.92,
          willChange: "transform",
        };
      }
      const idx = idsRef.current.indexOf(id);
      let shift = 0;
      if (
        drag.targetIndex > drag.originIndex && idx > drag.originIndex &&
        idx <= drag.targetIndex
      ) {
        shift = -drag.slot;
      } else if (
        drag.targetIndex < drag.originIndex &&
        idx >= drag.targetIndex &&
        idx < drag.originIndex
      ) {
        shift = drag.slot;
      }
      return {
        transform: `translateY(${String(shift)}px)`,
        transition: "transform 0.18s ease",
        position: "relative",
      };
    },
    [drag, dragOffsetX],
  );

  // During a drag, render in the stable server order (transforms show the
  // rearrangement); after drop, the optimistic order until the echo lands. Either
  // way, RECONCILE against the current ids so the rendered order is always a clean
  // permutation of what actually exists: drop ids no longer present, dedupe, and
  // append any new ids the base is missing. Without this a lingering optimistic
  // order could render a stale/duplicate row (3 rows while the list count says 2)
  // for a frame during the drop→server-echo window.
  const order = reconcile(drag ? ids : (optimistic ?? ids), ids);
  const publicDrag = drag
    ? {
      id: drag.id,
      originIndex: drag.originIndex,
      targetIndex: drag.targetIndex,
      depthSteps: drag.depthSteps,
      overId: drag.overId ?? null,
    }
    : null;
  return {
    order,
    draggingId: drag?.id ?? null,
    drag: publicDrag,
    registerItem,
    itemStyle,
    handleProps,
  };
}

// Project `base` onto the current `ids`: keep base's order for ids that still
// exist (deduped), then append any ids base didn't include. Guarantees the result
// is a duplicate-free permutation of exactly `ids`.
function reconcile(base: string[], ids: string[]): string[] {
  const live = new Set(ids);
  const seen = new Set<string>();
  const out: string[] = [];
  for (const id of base) {
    if (live.has(id) && !seen.has(id)) {
      seen.add(id);
      out.push(id);
    }
  }
  for (const id of ids) {
    if (!seen.has(id)) {
      seen.add(id);
      out.push(id);
    }
  }
  return out;
}
