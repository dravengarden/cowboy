import { useEffect } from "react";

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
    let raf = 0;
    const apply = (): void => {
      raf = 0;
      const overlap = Math.max(0, globalThis.innerHeight - (vv.offsetTop + vv.height));
      root.style.setProperty("--kb-inset", `${String(Math.round(overlap))}px`);
    };
    const onChange = (): void => {
      if (raf === 0) raf = globalThis.requestAnimationFrame(apply);
    };
    vv.addEventListener("resize", onChange);
    vv.addEventListener("scroll", onChange);
    apply();
    return () => {
      vv.removeEventListener("resize", onChange);
      vv.removeEventListener("scroll", onChange);
      if (raf !== 0) globalThis.cancelAnimationFrame(raf);
      root.style.removeProperty("--kb-inset");
    };
  }, []);
}
