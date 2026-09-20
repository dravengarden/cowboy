// Pointer/zoom/pan machinery for <ImageLightbox>, split out so the component
// file stays small and each function stays well under the line caps. All state
// is imperative (refs written straight to the DOM for 60fps gestures); the only
// React surface is the handlers this hook returns.

import { type RefObject, useCallback, useEffect, useRef } from "react";
import {
  constrainPanAxis,
  NO_DESTINATION_RESISTANCE,
  projectFlick,
  SWIPE_COMMIT_VELOCITY,
  trackPanVelocity,
} from "./image-lightbox-motion.ts";

export type LightboxMediaElement = HTMLImageElement | SVGSVGElement;

const MIN_SCALE = 1;
const MAX_SCALE = 6;
// Vertical drag (at 1x) past this many px releases into a dismiss.
const DISMISS_THRESHOLD = 110;
// Horizontal drag (at 1x) past this many px flips to the prev/next image.
const SWIPE_NAV_THRESHOLD = 70;
// Pointer PATH travel below this (px) counts as a tap, not a drag.
const TAP_SLOP = 8;
// …but a thumb tap on a phone often jitters well past 8px of total path while
// ending within a few px of where it started. So ALSO treat a release whose NET
// displacement (start→end) is below this as a tap — otherwise an imprecise tap
// on the backdrop falls through to "short drag → snap back" and never closes.
// Generous on purpose: a docs figure viewer is modal-like, so users expect a
// backdrop tap to dismiss even when their thumb rolls a fair bit; a deliberate
// drag (nav >70px, dismiss >110px) is still well clear of this.
const TAP_NET_SLOP = 44;
// Apple reserves double tap for zooming. A single image tap is intentionally a
// no-op so a finger lifted after inspecting/panning can never collapse the view.
const DOUBLE_TAP_MS = 300;
const DOUBLE_TAP_SLOP = 32;
const TAP_ZOOM_SCALE = 2.5;

export interface LightboxGesturesParams {
  imgRef: RefObject<LightboxMediaElement | null>;
  overlayRef: RefObject<HTMLDivElement | null>;
  /** Whether the lightbox is currently showing an image. */
  open: boolean;
  /** Source of the current image; a change resets the view. */
  src: string | null;
  canPrev: boolean;
  canNext: boolean;
  goPrev: () => void;
  goNext: () => void;
  onClose: () => void;
}

export interface LightboxGestures {
  // Bound to the OVERLAY (not the <img>), so pinch / pan / tap work over the
  // whole backdrop, not just on the image itself.
  onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerEnd: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerCancel: (e: React.PointerEvent<HTMLDivElement>) => void;
  onImageLoad: () => void;
  /** Step zoom toward the viewport centre (the dock +/− buttons). */
  zoomBy: (factor: number) => void;
}

const clamp = (s: number): number => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

export function useLightboxGestures(params: LightboxGesturesParams): LightboxGestures {
  const { imgRef, overlayRef, open, src, canPrev, canNext, goPrev, goNext, onClose } = params;

  // A zoomed image is deliberately promoted only after its final scale has
  // painted. Promoting it while the pinch is still changing scale makes iOS
  // cache a low-resolution texture; never promoting it makes every pan briefly
  // re-rasterize. Delayed promotion gives panning a stable, sharp final-scale
  // layer without adding latency to pointermove.
  const promoteTimer = useRef(0);
  // Live transform, applied imperatively for 60fps gestures.
  const tf = useRef({ scale: 1, x: 0, y: 0 });
  // At rest, the logical scale is baked into the element's CSS dimensions so
  // panning moves 1:1 rendered pixels instead of a scaled WebKit texture.
  const bakedScale = useRef(1);
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const geometry = useRef<
    {
      centerX: number;
      centerY: number;
      viewportWidth: number;
      viewportHeight: number;
      imageWidth: number;
      imageHeight: number;
      padding: number;
    } | null
  >(null);
  // True while a release animation is still interpolating the transform.
  const settling = useRef(false);
  const g = useRef({
    startX: 0,
    startY: 0,
    lastX: 0,
    lastY: 0,
    moved: 0,
    // Smoothed release speed (px/ms per axis) and the sample it was taken at.
    velX: 0,
    velY: 0,
    sampleAt: 0,
    // Raw finger travel at fit size. The painted x may be resisted when there
    // is no neighbour to swipe to, so the release decisions read the finger.
    fitX: 0,
    fitY: 0,
    pinchDist: 0,
    pinchScale: 1,
    pinchMidX: 0,
    pinchMidY: 0,
    pinched: false,
    onImage: false,
    panX: 0,
    panY: 0,
  });
  const lastImageTap = useRef({ at: 0, x: 0, y: 0 });

  const measureGeometry = useCallback(() => {
    const img = imgRef.current;
    const overlay = overlayRef.current;
    if (!img || !overlay) {
      geometry.current = null;
      return;
    }
    const rect = overlay.getBoundingClientRect();
    const mediaRect = img.getBoundingClientRect();
    const imageWidth = img instanceof HTMLImageElement
      ? img.offsetWidth / bakedScale.current
      : mediaRect.width / tf.current.scale;
    const imageHeight = img instanceof HTMLImageElement
      ? img.offsetHeight / bakedScale.current
      : mediaRect.height / tf.current.scale;
    // The plate's padding is part of the element's border box, so a transform
    // scales it along with the artwork while a baked layer would keep it at its
    // CSS size. Recover the unscaled value (a baked layer already carries
    // `padding × bakedScale`) so the bake can scale it to match.
    const padding = Number.parseFloat(
      globalThis.getComputedStyle(img).paddingTop || "0",
    ) / bakedScale.current;
    geometry.current = {
      centerX: rect.left + overlay.clientWidth / 2,
      centerY: rect.top + overlay.clientHeight / 2,
      viewportWidth: overlay.clientWidth,
      viewportHeight: overlay.clientHeight,
      padding: Number.isFinite(padding) ? padding : 0,
      // SVG has no offsetWidth/offsetHeight. Its client rect includes the
      // current transform, so divide out the logical scale to recover the
      // base layout size. Keep the established offset geometry for <img>.
      imageWidth,
      imageHeight,
    };
  }, [imgRef, overlayRef]);

  // Half the overflow of the zoomed figure on each axis: how far the pan may
  // travel before an edge comes into view. Zero means the figure already fits
  // that axis, and constrainPanAxis then holds it rigid there.
  const panBounds = useCallback((): { x: number; y: number } => {
    const box = geometry.current;
    if (!box || tf.current.scale <= 1) {
      return { x: 0, y: 0 };
    }
    return {
      x: Math.max(0, (box.imageWidth * tf.current.scale - box.viewportWidth) / 2),
      y: Math.max(0, (box.imageHeight * tf.current.scale - box.viewportHeight) / 2),
    };
  }, []);

  // Keep a zoomed image covering the viewport axis it exceeds. Without this a
  // pinch near an edge (or a fast follow-up pan) can leave the whole image
  // floating off-screen with no visual way to recover it.
  const constrainPan = useCallback((elastic = false) => {
    if (!geometry.current || tf.current.scale <= 1) {
      return;
    }
    const bounds = panBounds();
    tf.current.x = constrainPanAxis(tf.current.x, bounds.x, elastic);
    tf.current.y = constrainPanAxis(tf.current.y, bounds.y, elastic);
  }, [panBounds]);

  // `animate` is the default 0.22s settle (true), no transition (false), or an
  // explicit `transition` value — a released flick sizes its own coast, so its
  // duration is not a constant.
  const paintTransform = useCallback((animate: boolean | string = false, panLayer = false) => {
    const img = imgRef.current;
    if (!img) {
      return;
    }
    const { scale, x, y } = tf.current;
    const visualScale = scale / bakedScale.current;
    const transition = typeof animate === "string"
      ? animate
      : animate
      ? "transform 0.22s ease"
      : "none";
    settling.current = transition !== "none";
    img.style.transition = transition;
    img.style.transform = `translate(${x}px, ${y}px) scale(${visualScale})`;
    img.style.cursor = scale > 1 ? "grab" : "zoom-out";
    img.style.willChange = scale <= 1 || panLayer ? "transform" : "auto";
  }, [imgRef]);

  // Pointer events are already display-aligned by WebKit. Painting immediately
  // avoids adding a full frame of input latency; the operation is one transform
  // write and performs no layout reads.
  const applyTransform = useCallback((animate: boolean | string = false, panLayer = false) => {
    paintTransform(animate, panLayer);
  }, [paintTransform]);

  // A transition starts from the computed style of the LAST style recalculation.
  // Swapping the layer's layout size (bake / unbake) and then starting an
  // animated transform in the same task therefore interpolates the old
  // transform against the new layout — at 2x the figure flashes to 4x (or
  // collapses to fit) and rides that back over the whole settle. That is the
  // twitch at the end of a pinch. Committing the neutral, transition-less paint
  // first makes it the transition's starting value. One forced read per gesture
  // boundary; never inside a move.
  const commitPaint = useCallback(() => {
    imgRef.current?.getBoundingClientRect();
  }, [imgRef]);

  // A finger landing on a coasting figure must take it where it is, not where
  // it was headed: the transition owns the painted transform, while `tf` already
  // holds its destination. Freeze the interpolated value first or the next pan
  // starts from the target and the image jumps out from under the touch.
  const stopSettle = useCallback(() => {
    const img = imgRef.current;
    if (!img || !settling.current) {
      return;
    }
    // Adopt the painted value only once the transition has actually advanced.
    // Interrupting one in the same task that started it reads the frame it has
    // not left yet, which would rewind the figure to where the settle began —
    // a second zoom step tapped straight after the first lost that step.
    const advanced = img.getAnimations().some((animation) =>
      typeof animation.currentTime === "number" && animation.currentTime > 0
    );
    const painted = advanced ? globalThis.getComputedStyle(img).transform : "";
    if (painted && painted !== "none") {
      const matrix = new DOMMatrixReadOnly(painted);
      tf.current.x = matrix.m41;
      tf.current.y = matrix.m42;
      tf.current.scale = clamp(matrix.m11 * bakedScale.current);
    }
    paintTransform(false, tf.current.scale > 1);
    commitPaint();
  }, [imgRef, paintTransform, commitPaint]);

  const unbakeScale = useCallback(() => {
    const img = imgRef.current;
    const box = geometry.current;
    if (!img || !box || bakedScale.current === 1) {
      return;
    }
    img.style.width = `${box.imageWidth}px`;
    img.style.height = `${box.imageHeight}px`;
    img.style.maxWidth = "100%";
    img.style.maxHeight = "100%";
    img.style.padding = "";
    bakedScale.current = 1;
    // Repaint at the scale the layer was carrying: same pixels on screen, now
    // expressed as a transform again, and committed before any animation.
    paintTransform();
    commitPaint();
  }, [imgRef, paintTransform, commitPaint]);

  const bakePanLayer = useCallback(() => {
    const img = imgRef.current;
    const box = geometry.current;
    if (!img || !box || tf.current.scale <= 1 || pointers.current.size !== 0) {
      return;
    }
    // Bake the settled scale into layout dimensions. SVG text is then
    // rasterized at its displayed size and panning is a scale(1) transform,
    // avoiding the transient blur WebKit produces when moving a scaled
    // compositor texture.
    img.style.width = `${box.imageWidth * tf.current.scale}px`;
    img.style.height = `${box.imageHeight * tf.current.scale}px`;
    img.style.maxWidth = "none";
    img.style.maxHeight = "none";
    // Scale the plate's padding with the layer. Leaving it at its CSS size
    // keeps the border box right but widens the content box, so the artwork
    // jumped outwards (and up-left) on the frame the bake landed — the visible
    // twitch at the end of a pinch.
    img.style.padding = `${box.padding * tf.current.scale}px`;
    bakedScale.current = tf.current.scale;
    paintTransform(false, true);
    commitPaint();
  }, [imgRef, paintTransform, commitPaint]);

  const schedulePanLayer = useCallback((delay = 0) => {
    if (promoteTimer.current !== 0) {
      clearTimeout(promoteTimer.current);
    }
    promoteTimer.current = globalThis.setTimeout(() => {
      promoteTimer.current = 0;
      bakePanLayer();
    }, delay) as unknown as number;
  }, [bakePanLayer]);

  // Release a zoomed pan the way a scroll view does: coast the smoothed release
  // speed out on the compositor as ONE transition (never a per-frame JS spring),
  // clipped by the same bounds the drag honoured. A lift that carried no throw
  // just settles the elastic overshoot the finger was holding.
  const settlePan = useCallback((velocityX: number, velocityY: number) => {
    const flick = projectFlick(
      { x: tf.current.x, y: tf.current.y },
      { x: velocityX, y: velocityY },
      panBounds(),
    );
    if (!flick) {
      constrainPan();
      applyTransform(true, true);
      return;
    }
    tf.current.x = flick.x;
    tf.current.y = flick.y;
    applyTransform(`transform ${flick.durationMs}ms cubic-bezier(0.32, 0.72, 0, 1)`, true);
  }, [panBounds, constrainPan, applyTransform]);

  const setBackdrop = useCallback((dimAlpha: number) => {
    const o = overlayRef.current;
    if (o) {
      o.style.backgroundColor = `rgba(0, 0, 0, ${dimAlpha})`;
    }
  }, [overlayRef]);

  const reset = useCallback((animate = false) => {
    if (promoteTimer.current !== 0) {
      clearTimeout(promoteTimer.current);
      promoteTimer.current = 0;
    }
    // An animated return has to start from what is on screen, so give a baked
    // layer back to CSS sizing (repainted and committed) before the settle.
    if (animate) {
      unbakeScale();
    }
    const img = imgRef.current;
    if (img) {
      img.style.width = "";
      img.style.height = "";
      img.style.maxWidth = "100%";
      img.style.maxHeight = "100%";
      img.style.padding = "";
    }
    bakedScale.current = 1;
    tf.current = { scale: 1, x: 0, y: 0 };
    applyTransform(animate);
    setBackdrop(0.92);
  }, [imgRef, unbakeScale, applyTransform, setBackdrop]);

  // Zoom by `factor` keeping the viewport point (cx, cy) stationary.
  const zoomAt = useCallback((opts: {
    factor: number;
    cx: number;
    cy: number;
    animate?: boolean;
    elastic?: boolean;
  }) => {
    const box = geometry.current;
    if (!box) {
      return;
    }
    const prev = tf.current.scale;
    const next = clamp(prev * opts.factor);
    if (next === prev) {
      return;
    }
    if (promoteTimer.current !== 0) {
      clearTimeout(promoteTimer.current);
      promoteTimer.current = 0;
    }
    unbakeScale();
    // The untransformed image is flex-centred in the overlay. Derive its visual
    // centre from that stable box plus our live translation instead of reading
    // getBoundingClientRect(): during a pinch the midpoint translation has been
    // updated in `tf` but has not painted yet, so the DOM rect is one frame stale
    // and feeds a small compounding drift back into every scale step.
    const centerX = box.centerX + tf.current.x;
    const centerY = box.centerY + tf.current.y;
    const ratio = next / prev;
    tf.current.x += (opts.cx - centerX) * (1 - ratio);
    tf.current.y += (opts.cy - centerY) * (1 - ratio);
    tf.current.scale = next;
    if (next === MIN_SCALE) {
      tf.current.x = 0;
      tf.current.y = 0;
    } else {
      constrainPan(opts.elastic ?? false);
    }
    applyTransform(opts.animate ?? false);
  }, [applyTransform, constrainPan, unbakeScale]);

  const zoomBy = useCallback((factor: number) => {
    // A second step taken while the first is still easing would measure a
    // mid-transition rect: two quick taps on + landed at 1.5x of a figure that
    // had not finished growing, and left the pan bounds describing a size the
    // figure never had. Freeze the animation into the model first.
    stopSettle();
    measureGeometry();
    zoomAt({ factor, cx: globalThis.innerWidth / 2, cy: globalThis.innerHeight / 2, animate: true });
    schedulePanLayer(240);
  }, [stopSettle, measureGeometry, zoomAt, schedulePanLayer]);

  const settleGeometry = useCallback(() => {
    measureGeometry();
    if (tf.current.scale > 1) {
      constrainPan();
      applyTransform(true, true);
    } else if (tf.current.x !== 0 || tf.current.y !== 0) {
      reset(true);
    }
  }, [measureGeometry, constrainPan, applyTransform, reset]);

  // New image (open or navigation) → reset the view.
  useEffect(() => {
    if (src) {
      geometry.current = null;
      reset();
    }
  }, [src, reset]);

  // Wheel zoom (passive:false so we can preventDefault the page scroll). On the
  // OVERLAY so the wheel zooms from anywhere over the backdrop, not just the img.
  useEffect(() => {
    const overlay = overlayRef.current;
    if (!overlay || !open) {
      return;
    }
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      stopSettle();
      if (!geometry.current) {
        measureGeometry();
      }
      zoomAt({ factor: e.deltaY < 0 ? 1.18 : 1 / 1.18, cx: e.clientX, cy: e.clientY });
    };
    overlay.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      overlay.removeEventListener("wheel", onWheel);
    };
  }, [overlayRef, open, src, stopSettle, measureGeometry, zoomAt]);

  useEffect(() => {
    globalThis.addEventListener("resize", settleGeometry);
    return () => globalThis.removeEventListener("resize", settleGeometry);
  }, [settleGeometry]);

  // Clear any pending layer promotion on unmount.
  useEffect(() => () => {
    if (promoteTimer.current !== 0) {
      clearTimeout(promoteTimer.current);
    }
  }, []);

  const onPointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    const st = g.current;
    if (pointers.current.size === 1) {
      stopSettle();
      measureGeometry();
      st.startX = e.clientX;
      st.lastX = e.clientX;
      st.startY = e.clientY;
      st.lastY = e.clientY;
      st.moved = 0;
      st.velX = 0;
      st.velY = 0;
      st.sampleAt = performance.now();
      st.fitX = 0;
      st.fitY = 0;
      st.panX = tf.current.x;
      st.panY = tf.current.y;
      // Did the press land on the image (vs the backdrop)? Drives tap behaviour:
      // image → zoom toggle, backdrop → dismiss. getBoundingClientRect is the
      // VISUAL (transformed) box, so this is correct whether fit or zoomed.
      const ir = imgRef.current?.getBoundingClientRect();
      st.onImage = ir !== undefined && e.clientX >= ir.left &&
        e.clientX <= ir.right && e.clientY >= ir.top && e.clientY <= ir.bottom;
    } else if (pointers.current.size === 2) {
      unbakeScale();
      applyTransform();
      const [a, b] = [...pointers.current.values()];
      if (a && b) {
        st.pinchDist = Math.hypot(a.x - b.x, a.y - b.y);
        st.pinchScale = tf.current.scale;
        st.pinchMidX = (a.x + b.x) / 2;
        st.pinchMidY = (a.y + b.y) / 2;
        st.pinched = true;
      }
    }
  }, [imgRef, measureGeometry, stopSettle, unbakeScale, applyTransform]);

  const onPointerMove = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!pointers.current.has(e.pointerId)) {
      return;
    }
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    const st = g.current;

    if (pointers.current.size >= 2) {
      const [a, b] = [...pointers.current.values()];
      if (a && b && st.pinchDist > 0) {
        const dist = Math.hypot(a.x - b.x, a.y - b.y);
        const midX = (a.x + b.x) / 2;
        const midY = (a.y + b.y) / 2;
        // A pinch is scale AND translation: keep the content under the moving
        // midpoint instead of letting it drift away as both fingers travel.
        tf.current.x += midX - st.pinchMidX;
        tf.current.y += midY - st.pinchMidY;
        st.pinchMidX = midX;
        st.pinchMidY = midY;
        const target = clamp((st.pinchScale * dist) / st.pinchDist);
        zoomAt({ factor: target / tf.current.scale, cx: midX, cy: midY, elastic: true });
        st.panX = tf.current.x;
        st.panY = tf.current.y;
      }
      return;
    }

    const dx = e.clientX - st.lastX;
    const dy = e.clientY - st.lastY;
    st.lastX = e.clientX;
    st.lastY = e.clientY;
    st.moved += Math.abs(dx) + Math.abs(dy);
    const now = performance.now();
    st.velX = trackPanVelocity(st.velX, dx, now - st.sampleAt);
    st.velY = trackPanVelocity(st.velY, dy, now - st.sampleAt);
    st.sampleAt = now;

    if (tf.current.scale > 1) {
      // Pan the zoomed image.
      st.panX += dx;
      st.panY += dy;
      tf.current.x = st.panX;
      tf.current.y = st.panY;
      constrainPan(true);
      applyTransform(false, true);
    } else {
      // At fit: follow the finger on both axes. The release handler decides
      // whether the dominant axis means navigate (horizontal) or dismiss
      // (vertical). Only vertical travel fades the backdrop.
      st.fitX = e.clientX - st.startX;
      st.fitY = e.clientY - st.startY;
      // A single-figure preview has no neighbour to reach. Resist that axis
      // rather than sliding the whole figure away only to spring it back.
      const reachable = st.fitX > 0 ? canPrev : canNext;
      tf.current.x = reachable ? st.fitX : st.fitX * NO_DESTINATION_RESISTANCE;
      tf.current.y = st.fitY;
      applyTransform();
      setBackdrop(0.92 * (1 - Math.min(1, Math.abs(st.fitY) / 400)));
    }
  }, [zoomAt, applyTransform, constrainPan, setBackdrop, canPrev, canNext]);

  const onPointerEnd = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    pointers.current.delete(e.pointerId);
    if (pointers.current.size > 0) {
      // Rebase single-finger panning to the surviving pointer. Keeping the
      // pre-pinch lastX/lastY makes its next move look like a huge delta and is
      // the source of the post-pinch image jump.
      const remaining = pointers.current.values().next().value;
      if (remaining) {
        g.current.lastX = remaining.x;
        g.current.lastY = remaining.y;
        // The surviving finger starts a fresh pan: a velocity carried over from
        // the pinch would throw the figure the moment that finger lifts.
        g.current.velX = 0;
        g.current.velY = 0;
        g.current.sampleAt = performance.now();
      }
      return; // still pinching
    }
    const st = g.current;

    // A completed pinch is never a tap/dismiss gesture. Settle within bounds
    // and wait for the next independent pointer sequence.
    if (st.pinched) {
      st.pinched = false;
      if (tf.current.scale <= 1) {
        reset();
        return;
      }
      // Convert to full-resolution CSS dimensions before any release
      // animation. The elastic correction below then animates only a scale(1)
      // translation, so lifting the fingers cannot expose a blurry scaled
      // texture for the duration of the snap-back. The bake commits its own
      // paint, so that translation is all this transition has to interpolate.
      bakePanLayer();
      constrainPan();
      applyTransform(true, true);
      return;
    }

    if (tf.current.scale <= 1) {
      const { fitX: x, fitY: y } = st;
      // A quick flick commits on speed alone even when it never travelled the
      // threshold distance — the rule the drawers and the product pager use.
      // The small-travel guard keeps a jittery tap out of it.
      const flicked = (velocity: number, travel: number): boolean =>
        Math.abs(velocity) >= SWIPE_COMMIT_VELOCITY && Math.abs(travel) > TAP_SLOP;
      // Horizontal swipe wins when it dominates → previous / next image.
      if (
        Math.abs(x) > Math.abs(y) &&
        (Math.abs(x) > SWIPE_NAV_THRESHOLD || flicked(st.velX, x))
      ) {
        const moved = x > 0 ? canPrev : canNext;
        if (moved) {
          if (x > 0) {
            goPrev();
          } else {
            goNext();
          }
          return; // index change resets the view
        }
        reset(true); // at an end — rubber-band back
        return;
      }
      // Vertical drag far enough — or thrown fast enough → dismiss.
      if (Math.abs(y) > DISMISS_THRESHOLD || flicked(st.velY, y)) {
        onClose();
        return;
      }
    }

    // Tap = small path OR small net finger displacement (start→end). The net
    // check rescues jittery thumb taps that drift past TAP_SLOP yet land where
    // they began — the common "tap the backdrop to close" gesture.
    const netMove = Math.hypot(e.clientX - st.startX, e.clientY - st.startY);
    if (st.moved < TAP_SLOP || netMove < TAP_NET_SLOP) {
      // A tap on the image only zooms when it completes a double tap. This
      // follows the iOS convention and prevents an ordinary finger lift while
      // inspecting a zoomed diagram from unexpectedly collapsing it.
      if (st.onImage) {
        const now = performance.now();
        const previous = lastImageTap.current;
        const isDoubleTap = now - previous.at <= DOUBLE_TAP_MS &&
          Math.hypot(e.clientX - previous.x, e.clientY - previous.y) <= DOUBLE_TAP_SLOP;
        lastImageTap.current = { at: now, x: e.clientX, y: e.clientY };
        if (isDoubleTap) {
          lastImageTap.current.at = 0;
          if (tf.current.scale > 1) {
            reset(true);
          } else {
            zoomAt({ factor: TAP_ZOOM_SCALE, cx: e.clientX, cy: e.clientY, animate: true });
            schedulePanLayer(240);
          }
        } else if (tf.current.scale > 1) {
          settlePan(st.velX, st.velY);
        }
      } else {
        onClose();
      }
      return;
    }

    // A short drag that didn't dismiss / navigate → snap back to fit. A zoomed
    // drag coasts its release speed out and settles any elastic margin with it.
    if (tf.current.scale <= 1) {
      reset(true);
    } else {
      settlePan(st.velX, st.velY);
    }
  }, [
    onClose,
    reset,
    zoomAt,
    constrainPan,
    applyTransform,
    settlePan,
    bakePanLayer,
    schedulePanLayer,
    canPrev,
    canNext,
    goPrev,
    goNext,
  ]);

  const onPointerCancel = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!pointers.current.has(e.pointerId)) {
      return;
    }
    pointers.current.delete(e.pointerId);
    const remaining = pointers.current.values().next().value;
    if (remaining) {
      g.current.lastX = remaining.x;
      g.current.lastY = remaining.y;
      return;
    }
    g.current.pinched = false;
    if (tf.current.scale <= 1) {
      reset(true);
    } else {
      constrainPan();
      applyTransform(true, true);
      setBackdrop(0.92);
    }
  }, [reset, constrainPan, applyTransform, setBackdrop]);

  return { onPointerDown, onPointerMove, onPointerEnd, onPointerCancel, onImageLoad: settleGeometry, zoomBy };
}
