import {
  type RefCallback,
  type RefObject,
  useCallback,
  useEffect,
  useRef,
} from "react";

const GEOMETRY_SETTLE_DELAYS_MS = [120, 300, 550] as const;

function borderBoxHeight(element: HTMLElement | null): number {
  // offsetHeight is the untransformed border box. The Mobile Sessions drawer
  // translates/scales the whole surface during a gesture; geometry must remain
  // tied to layout, not to that temporary compositor transform.
  return element?.offsetHeight ?? 0;
}

function px(value: number): string {
  return `${String(value)}px`;
}

/**
 * Owns every dimension shared by the floating Composer stack, its frosted
 * material, and Transcript clearance.
 *
 * The live material follows every border-box change. Transcript clearance is
 * held during MUI disclosure motion and published once at the settled edge, so
 * a long column-reverse transcript is not re-laid-out every animation frame.
 */
export function useFloatingComposerGeometry({
  surfaceRef,
  navbarAtBottom,
}: {
  surfaceRef: RefObject<HTMLDivElement | null>;
  navbarAtBottom: boolean;
}): {
  appBarRef: RefCallback<HTMLElement>;
  composerRef: RefCallback<HTMLElement>;
} {
  const observerRef = useRef<ResizeObserver | null>(null);
  const navbarAtBottomRef = useRef(navbarAtBottom);
  navbarAtBottomRef.current = navbarAtBottom;
  const appBarElementRef = useRef<HTMLElement | null>(null);
  const composerElementRef = useRef<HTMLElement | null>(null);
  const activeDisclosureTransitionsRef = useRef(new Set<EventTarget>());
  const pendingTranscriptInsetRef = useRef("0px");
  const settleTimersRef = useRef<number[]>([]);
  const disclosureSettleTimerRef = useRef(0);
  const measurementFrameRef = useRef(0);

  const publishTranscriptInset = useCallback((): void => {
    surfaceRef.current?.style.setProperty(
      "--transcript-bottom-inset",
      pendingTranscriptInsetRef.current,
    );
  }, [surfaceRef]);

  const measure = useCallback((): void => {
    const surface = surfaceRef.current;
    if (!surface) return;

    const navbarHeight = borderBoxHeight(appBarElementRef.current);
    const composerHeight = borderBoxHeight(composerElementRef.current);
    const floatingStackHeight = composerHeight +
      (navbarAtBottomRef.current ? navbarHeight : 0);

    surface.style.setProperty("--navbar-h", px(navbarHeight));
    surface.style.setProperty("--floating-stack-h", px(floatingStackHeight));
    pendingTranscriptInsetRef.current = px(floatingStackHeight);
    if (activeDisclosureTransitionsRef.current.size === 0) {
      publishTranscriptInset();
    }
  }, [publishTranscriptInset, surfaceRef]);

  const clearSettledMeasurements = useCallback((): void => {
    for (const timer of settleTimersRef.current) globalThis.clearTimeout(timer);
    settleTimersRef.current = [];
  }, []);

  const scheduleSettledMeasurement = useCallback((): void => {
    measure();
    if (measurementFrameRef.current !== 0) {
      globalThis.cancelAnimationFrame(measurementFrameRef.current);
    }
    measurementFrameRef.current = globalThis.requestAnimationFrame((): void => {
      measurementFrameRef.current = 0;
      measure();
    });
    clearSettledMeasurements();
    settleTimersRef.current = GEOMETRY_SETTLE_DELAYS_MS.map((delay) =>
      globalThis.setTimeout(measure, delay)
    );
  }, [clearSettledMeasurements, measure]);

  const disclosureTransition = useCallback((event: TransitionEvent): void => {
    const target = event.target;
    if (
      event.propertyName !== "height" ||
      !(target instanceof HTMLElement) ||
      !target.classList.contains("MuiCollapse-root")
    ) return;
    if (event.type === "transitionrun") {
      activeDisclosureTransitionsRef.current.add(target);
      if (disclosureSettleTimerRef.current !== 0) {
        globalThis.clearTimeout(disclosureSettleTimerRef.current);
      }
      // An interrupted/unmounted MUI Collapse does not always deliver a final
      // transitioncancel in WebKit. Never let that strand Transcript on the old
      // inset; the longest disclosure motion is well below this bound.
      disclosureSettleTimerRef.current = globalThis.setTimeout((): void => {
        disclosureSettleTimerRef.current = 0;
        activeDisclosureTransitionsRef.current.clear();
        measure();
      }, 700);
      return;
    }
    activeDisclosureTransitionsRef.current.delete(target);
    if (activeDisclosureTransitionsRef.current.size === 0) {
      if (disclosureSettleTimerRef.current !== 0) {
        globalThis.clearTimeout(disclosureSettleTimerRef.current);
        disclosureSettleTimerRef.current = 0;
      }
      measure();
    }
  }, [measure]);

  const observe = useCallback(
    (slot: "appbar" | "composer", element: HTMLElement | null): void => {
      observerRef.current ??= new ResizeObserver(measure);
      const observer = observerRef.current;
      const previous = slot === "appbar"
        ? appBarElementRef.current
        : composerElementRef.current;

      if (previous) observer.unobserve(previous);
      if (slot === "composer" && previous) {
        previous.removeEventListener("transitionrun", disclosureTransition, true);
        previous.removeEventListener("transitionend", disclosureTransition, true);
        previous.removeEventListener("transitioncancel", disclosureTransition, true);
        activeDisclosureTransitionsRef.current.clear();
      }

      if (slot === "appbar") appBarElementRef.current = element;
      else composerElementRef.current = element;

      if (element) {
        // iOS safe-area changes alter padding without altering the content box.
        // Observing the default content-box left --navbar-h stale after keyboard
        // dismissal (44px while the real border box was 60px).
        try {
          observer.observe(element, { box: "border-box" });
        } catch {
          observer.observe(element);
        }
        if (slot === "composer") {
          element.addEventListener("transitionrun", disclosureTransition, true);
          element.addEventListener("transitionend", disclosureTransition, true);
          element.addEventListener("transitioncancel", disclosureTransition, true);
        }
      }
      scheduleSettledMeasurement();
    },
    [disclosureTransition, measure, scheduleSettledMeasurement],
  );

  const appBarRef = useCallback<RefCallback<HTMLElement>>(
    (element) => observe("appbar", element),
    [observe],
  );
  const composerRef = useCallback<RefCallback<HTMLElement>>(
    (element) => observe("composer", element),
    [observe],
  );

  useEffect(() => {
    const viewport = globalThis.visualViewport;
    const document = globalThis.document;
    const onViewportChange = (): void => scheduleSettledMeasurement();

    viewport?.addEventListener("resize", onViewportChange);
    viewport?.addEventListener("scroll", onViewportChange);
    globalThis.addEventListener("resize", onViewportChange);
    document.addEventListener("focusin", onViewportChange);
    document.addEventListener("focusout", onViewportChange);
    scheduleSettledMeasurement();

    return () => {
      viewport?.removeEventListener("resize", onViewportChange);
      viewport?.removeEventListener("scroll", onViewportChange);
      globalThis.removeEventListener("resize", onViewportChange);
      document.removeEventListener("focusin", onViewportChange);
      document.removeEventListener("focusout", onViewportChange);
      clearSettledMeasurements();
      if (measurementFrameRef.current !== 0) {
        globalThis.cancelAnimationFrame(measurementFrameRef.current);
        measurementFrameRef.current = 0;
      }
    };
  }, [clearSettledMeasurements, scheduleSettledMeasurement]);

  useEffect(() => (): void => {
    observerRef.current?.disconnect();
    const composer = composerElementRef.current;
    if (composer) {
      composer.removeEventListener("transitionrun", disclosureTransition, true);
      composer.removeEventListener("transitionend", disclosureTransition, true);
      composer.removeEventListener("transitioncancel", disclosureTransition, true);
    }
    clearSettledMeasurements();
    if (measurementFrameRef.current !== 0) {
      globalThis.cancelAnimationFrame(measurementFrameRef.current);
    }
    if (disclosureSettleTimerRef.current !== 0) {
      globalThis.clearTimeout(disclosureSettleTimerRef.current);
    }
  }, [clearSettledMeasurements, disclosureTransition]);

  return { appBarRef, composerRef };
}
