import { useEffect, useState } from "react";

// Publish the on-screen keyboard's overlap of the layout viewport as the
// `--kb-inset` CSS var (px). The composer column lifts by this so it clears the
// keyboard AND the iOS-native accessory / IME-candidate bar above it (that bar
// is part of the system keyboard and can't be removed in a pure web app — see
// the composer's CodeMirror history).
//
// Why visualViewport, not `interactive-widget=resizes-content` alone: iOS Safari
// doesn't reliably shrink the LAYOUT viewport for the keyboard, and never for
// that extra bar, so the fixed body keeps full height and the keyboard chrome
// covers the bottom. visualViewport reports the actual visible region, so the
// overlap = layout-viewport bottom − visual-viewport bottom. When the layout
// viewport DID shrink (Android / a supporting engine), innerHeight shrinks too
// and the overlap self-zeroes — no double lift.
//
// Imperative (sets the CSS var, no React state) so the keyboard's open/close
// animation doesn't re-render the tree every frame; a rAF coalesces bursts.
export function useKeyboardInset(): void {
  useEffect(() => {
    const vv = globalThis.visualViewport;
    if (!vv) return undefined;
    const root = globalThis.document.documentElement;
    const doc = globalThis.document;
    let raf = 0;
    let timers: number[] = [];
    const apply = (): void => {
      raf = 0;
      const overlap = Math.max(0, globalThis.innerHeight - (vv.offsetTop + vv.height));
      root.style.setProperty("--kb-inset", `${String(Math.round(overlap))}px`);
    };
    const applyNow = (): void => {
      if (raf === 0) raf = globalThis.requestAnimationFrame(apply);
    };
    const clearTimers = (): void => {
      for (const t of timers) globalThis.clearTimeout(t);
      timers = [];
    };
    // iOS fires visualViewport `resize` DURING the keyboard's open/close
    // animation, but the FINAL settled frame is routinely missed — leaving
    // --kb-inset at a mid-animation (often near-zero) value. A sheet that opens
    // and focuses its field then renders its footer BEHIND the keyboard until the
    // next event (the "float bar only appears once you start typing" bug). So on a
    // keyboard-changing event re-measure NOW and again after the animation settles
    // (~120/300/550ms). focusin/focusout also re-measure: focusing a field is what
    // raises the keyboard, and its resize can land before the field settles.
    const schedule = (): void => {
      applyNow();
      clearTimers();
      timers = [120, 300, 550].map((d) => globalThis.setTimeout(apply, d));
    };
    vv.addEventListener("resize", schedule);
    // `scroll` fires every scroll frame and never changes the keyboard height, so
    // it only needs the cheap immediate re-measure — not the settle timers.
    vv.addEventListener("scroll", applyNow);
    doc.addEventListener("focusin", schedule);
    doc.addEventListener("focusout", schedule);
    schedule();
    return () => {
      vv.removeEventListener("resize", schedule);
      vv.removeEventListener("scroll", applyNow);
      doc.removeEventListener("focusin", schedule);
      doc.removeEventListener("focusout", schedule);
      clearTimers();
      if (raf !== 0) globalThis.cancelAnimationFrame(raf);
      root.style.removeProperty("--kb-inset");
    };
  }, []);
}

// Reactive "is the on-screen keyboard open?" — true when it overlaps the layout
// viewport by more than a flicker threshold. Drives the compose sheet's
// full-screen ↔ content-height switch: full-screen while typing, snug when the
// keyboard is dismissed (no stranded bar over an empty canvas). Re-measures on
// the same signals as useKeyboardInset, incl. focus changes + the settle timers,
// so it flips the moment the keyboard finishes animating.
export function useKeyboardOpen(): boolean {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const vv = globalThis.visualViewport;
    if (!vv) return undefined;
    const doc = globalThis.document;
    let raf = 0;
    let timers: number[] = [];
    const apply = (): void => {
      raf = 0;
      const overlap = globalThis.innerHeight - (vv.offsetTop + vv.height);
      // ≥120px so a stray inset (notch toolbar, rubber-band) never reads as a
      // keyboard; a real keyboard overlaps far more.
      setOpen(overlap > 120);
    };
    const applyNow = (): void => {
      if (raf === 0) raf = globalThis.requestAnimationFrame(apply);
    };
    const clearTimers = (): void => {
      for (const t of timers) globalThis.clearTimeout(t);
      timers = [];
    };
    const schedule = (): void => {
      applyNow();
      clearTimers();
      timers = [120, 300, 550].map((d) => globalThis.setTimeout(apply, d));
    };
    vv.addEventListener("resize", schedule);
    vv.addEventListener("scroll", applyNow);
    doc.addEventListener("focusin", schedule);
    doc.addEventListener("focusout", schedule);
    schedule();
    return () => {
      vv.removeEventListener("resize", schedule);
      vv.removeEventListener("scroll", applyNow);
      doc.removeEventListener("focusin", schedule);
      doc.removeEventListener("focusout", schedule);
      clearTimers();
      if (raf !== 0) globalThis.cancelAnimationFrame(raf);
    };
  }, []);
  return open;
}
