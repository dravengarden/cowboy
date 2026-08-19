import { useEffect, useRef, useState } from "react";
import {
  inferKeyboardOpen,
  isUnreliableVisualViewport,
  keyboardCoverOverlap,
  fixedLayoutHeight,
  paintedLayoutHeight,
  shouldLearnKeyboardFreeBaseline,
  visualViewportBox,
} from "./keyboardGeometry.ts";
import { isMobileEditorFocusTransferPending } from "./composer/mobileComposerFocus";
import { isNativeShell } from "./nativeShell";

// Publish the on-screen keyboard's overlap of the layout viewport as the
// `--kb-inset` CSS var (px). Browser/PWA surfaces use it to clear the keyboard
// and the iOS-native accessory / IME-candidate bar. The Tauri shell skips it
// because its native layer already resizes the WKWebView.
//
// Why visualViewport, not `interactive-widget=resizes-content` alone: iOS Safari
// doesn't reliably shrink the LAYOUT viewport for the keyboard, and never for
// that extra bar, so the painted page can stay full height and the keyboard
// chrome covers the bottom. Measure the painted html/#root box against
// visualViewport.height. window.innerHeight is not that box: it often stays on
// the pre-keyboard layout after resizes-content or Safari's compact URL bar
// have already shortened the page, and padding from that stale height leaves
// a lavender band above chrome that is already outside the webview.
//
// Imperative (sets the CSS var, no React state) so the keyboard's open/close
// animation doesn't re-render the tree every frame; a rAF coalesces bursts.
export function useKeyboardInset(): void {
  useEffect(() => {
    // In the native shell the WebView itself is resized for the keyboard (the
    // Capacitor resize:native model — CowboyNativeTweaks.mm shrinks the WKWebView
    // frame), so the native side ALREADY lifts the composer clear of the keyboard.
    // visualViewport ALSO reports the overlap inside WKWebView (it fires here,
    // unlike `interactive-widget`), so publishing --kb-inset on top would lift the
    // composer a SECOND time → a keyboard-height blank gap between the composer and
    // the keyboard (observed on device). Native resize is the sole avoidance in the
    // shell; --kb-inset stays 0 so `bottom/pb: var(--kb-inset, 0px)` collapse to the
    // native-resized bottom. Browser surfaces continue to use visualViewport.
    if (isNativeShell()) return undefined;
    const vv = globalThis.visualViewport;
    if (!vv) return undefined;
    const root = globalThis.document.documentElement;
    const doc = globalThis.document;
    let raf = 0;
    let timers: number[] = [];
    let poll = 0;
    let lastInset = -1;
    let lastVvHeight = -1;
    let lastVvOffset = -1;
    const apply = (): void => {
      raf = 0;
      // Keyboard overlap = how much of the painted page still sits *below*
      // the visual viewport. Subtract clamped offsetTop so a Safari pan to
      // keep the field on screen is not counted as cover; rubber-band
      // spikes are clamped and cannot inflate the inset.
      const rootHeight = doc.getElementById("root")?.clientHeight ?? 0;
      const layoutHeight = paintedLayoutHeight(
        globalThis.innerHeight,
        doc.documentElement.clientHeight,
        rootHeight,
      );
      if (isUnreliableVisualViewport(layoutHeight, vv.height)) return;
      // Cover sheets (New Session) are position:fixed against html's box,
      // which stays tall on Safari tabs. Pin them with --vv-* so Title
      // cannot pan off the top of a 100dvh cover.
      const coverBox = visualViewportBox(
        fixedLayoutHeight(
          doc.documentElement.clientHeight,
          rootHeight,
          globalThis.innerHeight,
        ),
        vv.height,
        vv.offsetTop,
      );
      if (coverBox.height !== lastVvHeight) {
        lastVvHeight = coverBox.height;
        root.style.setProperty("--vv-height", `${String(coverBox.height)}px`);
      }
      if (coverBox.offset !== lastVvOffset) {
        lastVvOffset = coverBox.offset;
        root.style.setProperty("--vv-offset", `${String(coverBox.offset)}px`);
      }
      // Do not add the iOS form accessory (∧ ∨ ✓) here. On PWA,
      // resizes-content already parks the painted page at the keyboard
      // top; the accessory sits below the visual viewport. Folding 44px
      // into --kb-inset left an empty band between the composer and the
      // bar. New Session clears that bar by being a cover sheet, not by
      // padding the whole app.
      // Safari tabs (not PWA/app): after resizes-content the painted box
      // already excludes the keyboard. visualViewport is then shorter by
      // the compact URL pill, often with offsetTop = 0. keyboardCoverOverlap
      // drops that chrome-sized remainder so we do not pad a lavender band
      // above cowboy.stormbird.xyz. PWA remainder is ~0; native skips this.
      const overlap = keyboardCoverOverlap(
        layoutHeight,
        vv.height,
        vv.offsetTop,
      );
      // Only write on change — the focus poll below runs apply() every 300ms, and
      // a same-value setProperty would still be a needless style touch each tick.
      if (overlap !== lastInset) {
        lastInset = overlap;
        root.style.setProperty("--kb-inset", `${String(overlap)}px`);
      }
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
    // Poll WHILE a field is focused. iOS changes the keyboard's height — the
    // IME candidate / predictive bar appearing or disappearing as you type, a
    // layout switch — WITHOUT reliably firing a visualViewport `resize`, so
    // --kb-inset goes stale (too big) and the cover sheet sits with a gap above the
    // keyboard ("还是有一定概率出现"). A 300ms re-measure while focused self-corrects
    // any untracked change; the change-guard above keeps idle ticks free.
    const startPoll = (): void => {
      if (poll === 0) poll = globalThis.setInterval(apply, 300);
    };
    const stopPoll = (): void => {
      if (poll !== 0) {
        globalThis.clearInterval(poll);
        poll = 0;
      }
    };
    const onFocusIn = (): void => {
      schedule();
      startPoll();
    };
    const onFocusOut = (): void => {
      schedule();
      stopPoll();
    };
    vv.addEventListener("resize", schedule);
    // `scroll` fires every scroll frame and never changes the keyboard height, so
    // it only needs the cheap immediate re-measure — not the settle timers.
    vv.addEventListener("scroll", applyNow);
    doc.addEventListener("focusin", onFocusIn);
    doc.addEventListener("focusout", onFocusOut);
    schedule();
    // A field may already be focused before this effect mounts (the sheet auto-
    // focuses its editor on open).
    if (doc.activeElement !== null && doc.activeElement !== doc.body) startPoll();
    return () => {
      vv.removeEventListener("resize", schedule);
      vv.removeEventListener("scroll", applyNow);
      doc.removeEventListener("focusin", onFocusIn);
      doc.removeEventListener("focusout", onFocusOut);
      clearTimers();
      stopPoll();
      if (raf !== 0) globalThis.cancelAnimationFrame(raf);
      root.style.removeProperty("--kb-inset");
      root.style.removeProperty("--vv-height");
      root.style.removeProperty("--vv-offset");
    };
  }, []);
}

function hasEditableFocus(doc: Document): boolean {
  const active = doc.activeElement;
  if (!(active instanceof Element)) return false;
  return active.matches("input, textarea, [contenteditable='true']") ||
    active.closest("[contenteditable='true']") !== null;
}

// Reactive "is the on-screen keyboard open?" — true when it overlaps the layout
// viewport by more than a flicker threshold. Drives the compose sheet's
// full-screen ↔ content-height switch: full-screen while typing, snug when the
// keyboard is dismissed (no stranded bar over an empty canvas). Re-measures on
// the same signals as useKeyboardInset, incl. focus changes + the settle timers,
// so it flips the moment the keyboard finishes animating.
export function useKeyboardOpen(): boolean {
  const [open, setOpen] = useState(false);
  const baselineHeightRef = useRef(0);
  useEffect(() => {
    const vv = globalThis.visualViewport;
    if (!vv) return undefined;
    const doc = globalThis.document;
    let raf = 0;
    let timers: number[] = [];
    let poll = 0;
    const apply = (): void => {
      raf = 0;
      const layoutHeight = globalThis.innerHeight;
      const visualHeight = vv.height;
      if (isUnreliableVisualViewport(layoutHeight, visualHeight)) return;
      const editableFocused = hasEditableFocus(doc);
      const visibleHeight = Math.min(layoutHeight, visualHeight);
      if (baselineHeightRef.current === 0) {
        baselineHeightRef.current = Math.max(layoutHeight, visualHeight);
      }
      const next = inferKeyboardOpen({
        layoutHeight,
        visualHeight,
        baselineHeight: baselineHeightRef.current,
        editableFocused,
      }) || isMobileEditorFocusTransferPending();
      setOpen(next);
      // Never learn the shrunken keyboard viewport as the baseline. Expand →
      // collapse remounts the editor and drops focus for a frame; treating
      // that keyboard-sized height as the new rest height makes later
      // focused frames look like "keyboard closed". Rotation reseeds below.
      if (
        !editableFocused && !next &&
        shouldLearnKeyboardFreeBaseline(
          baselineHeightRef.current,
          visibleHeight,
        )
      ) {
        baselineHeightRef.current = visibleHeight;
      } else if (next || editableFocused) {
        baselineHeightRef.current = Math.max(
          baselineHeightRef.current,
          visibleHeight,
        );
      }
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
    const startPoll = (): void => {
      if (poll === 0) poll = globalThis.setInterval(apply, 250);
    };
    const stopPoll = (): void => {
      if (poll !== 0) {
        globalThis.clearInterval(poll);
        poll = 0;
      }
    };
    const onFocusIn = (): void => {
      schedule();
      startPoll();
    };
    const onFocusOut = (): void => {
      schedule();
      stopPoll();
    };
    const onOrientation = (): void => {
      baselineHeightRef.current = 0;
      schedule();
    };
    vv.addEventListener("resize", schedule);
    vv.addEventListener("scroll", applyNow);
    globalThis.addEventListener("resize", schedule);
    globalThis.addEventListener("orientationchange", onOrientation);
    doc.addEventListener("focusin", onFocusIn);
    doc.addEventListener("focusout", onFocusOut);
    schedule();
    if (hasEditableFocus(doc)) startPoll();
    return () => {
      vv.removeEventListener("resize", schedule);
      vv.removeEventListener("scroll", applyNow);
      globalThis.removeEventListener("resize", schedule);
      globalThis.removeEventListener("orientationchange", onOrientation);
      doc.removeEventListener("focusin", onFocusIn);
      doc.removeEventListener("focusout", onFocusOut);
      clearTimers();
      stopPoll();
      if (raf !== 0) globalThis.cancelAnimationFrame(raf);
    };
  }, []);
  return open;
}
