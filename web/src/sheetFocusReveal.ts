import { useEffect } from "react";
import { KEYBOARD_INSET_CHANGED_EVENT } from "./keyboardInset";
import {
  closestScrollableSettingsSurface,
  settingsFocusRevealDelta,
} from "./settingsDrillInScroll";

/** A plain form field. Editors own their caret reveal: CodeMirror and the
 * composer's native textarea are coupled to iOS IME and caret behaviour
 * (mdlive/PITFALLS.md), so they are never scrolled from here. */
export function isSheetFormField(target: EventTarget | null): target is HTMLElement {
  if (!(target instanceof HTMLInputElement) && !(target instanceof HTMLTextAreaElement)) {
    return false;
  }
  if (target.closest(".cm-editor, [data-mobile-native-textarea], [data-mobile-native-editor]")) {
    return false;
  }
  return !(target instanceof HTMLInputElement) || ![
    "button",
    "checkbox",
    "color",
    "file",
    "hidden",
    "radio",
    "range",
    "reset",
    "submit",
  ].includes(target.type);
}

/** Scroll a focused field inside a sheet's own scroller so it sits between the
 * scroller's top and the sheet footer, returning the distance scrolled. The
 * sheet shrinks above the keyboard only after the platform's own reveal ran,
 * and that reveal moves the document, not the sheet's scroller. */
export function revealSheetField(field: HTMLElement): number {
  const sheet = field.closest<HTMLElement>("[data-detent-sheet='true']");
  if (!sheet || field.closest("[data-detent-sheet-footer]")) return 0;
  // App Settings reveals its fields beneath its own navigation (App.tsx).
  if (sheet.querySelector("[data-mobile-settings-navigation]")) return 0;
  const surface = closestScrollableSettingsSurface(field.parentElement ?? field);
  if (!sheet.contains(surface) || surface === field) return 0;
  const surfaceBounds = surface.getBoundingClientRect();
  const footer = sheet.querySelector<HTMLElement>("[data-detent-sheet-footer]");
  const footerTop = footer && footer.getClientRects().length > 0
    ? footer.getBoundingClientRect().top
    : surfaceBounds.bottom;
  const viewportBottom = globalThis.visualViewport
    ? globalThis.visualViewport.offsetTop + globalThis.visualViewport.height
    : surfaceBounds.bottom;
  const box = (field.closest<HTMLElement>(".MuiFormControl-root") ?? field)
    .getBoundingClientRect();
  const delta = settingsFocusRevealDelta(box, {
    top: surfaceBounds.top,
    bottom: Math.min(surfaceBounds.bottom, footerTop, viewportBottom),
  });
  if (Math.abs(delta) < 0.5) return 0;
  surface.scrollBy({ top: delta, behavior: "auto" });
  return delta;
}

/** Mount once at the app root: keep a focused sheet field visible while the
 * keyboard opens, resizes or the sheet relayouts around it. */
export function useSheetFocusReveal(): void {
  useEffect(() => {
    const doc = globalThis.document;
    const viewport = globalThis.visualViewport;
    let raf = 0;
    let timers: number[] = [];
    const reveal = (): void => {
      raf = 0;
      const active = doc.activeElement;
      if (isSheetFormField(active)) revealSheetField(active);
    };
    const schedule = (): void => {
      if (!isSheetFormField(doc.activeElement)) return;
      if (raf !== 0) globalThis.cancelAnimationFrame(raf);
      raf = globalThis.requestAnimationFrame(reveal);
      for (const timer of timers) globalThis.clearTimeout(timer);
      // The keyboard animates for ~250ms and the sheet follows it.
      timers = [120, 300, 550].map((delay) => globalThis.setTimeout(reveal, delay));
    };
    doc.addEventListener("focusin", schedule);
    viewport?.addEventListener("resize", schedule);
    globalThis.addEventListener("resize", schedule);
    globalThis.addEventListener(KEYBOARD_INSET_CHANGED_EVENT, schedule);
    return () => {
      doc.removeEventListener("focusin", schedule);
      viewport?.removeEventListener("resize", schedule);
      globalThis.removeEventListener("resize", schedule);
      globalThis.removeEventListener(KEYBOARD_INSET_CHANGED_EVENT, schedule);
      for (const timer of timers) globalThis.clearTimeout(timer);
      if (raf !== 0) globalThis.cancelAnimationFrame(raf);
    };
  }, []);
}
