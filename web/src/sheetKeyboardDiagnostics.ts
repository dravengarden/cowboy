import { useEffect } from "react";
import { isNativeShell } from "./nativeShell";
import { reportClientLog } from "./observability";

// Field diagnostics for a compact sheet that owns a text field on a touch
// device. A physical iPad (Home Screen web app, third-party split keyboard)
// kept the folder name prompt behind the keyboard and left the page one
// keyboard height too high after it closed (2026-09-17); neither the iPad
// Simulator's Safari tab nor its standalone web app reproduces that with
// Apple's keyboards, in any sheet placement. These few geometry samples per
// prompt turn the next occurrence into numbers. They carry layout metrics
// only, never field content.

const SAMPLE_DELAYS_MS = [0, 450, 1300];
let reportsLeft = 12;

function px(value: number | undefined | null): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.round(value)
    : -1;
}

export function sheetKeyboardGeometry(
  phase: string,
  sheet: Element | null,
): Record<string, string | number | boolean> {
  const doc = globalThis.document;
  const vv = globalThis.visualViewport;
  const root = doc.getElementById("root");
  const rect = sheet?.getBoundingClientRect();
  const active = doc.activeElement;
  const rootStyle = globalThis.getComputedStyle(doc.documentElement);
  return {
    phase,
    native_shell: isNativeShell(),
    standalone:
      globalThis.matchMedia?.("(display-mode: standalone)").matches === true,
    inner_height: px(globalThis.innerHeight),
    client_height: px(doc.documentElement.clientHeight),
    root_height: px(root?.clientHeight),
    vv_height: px(vv?.height),
    vv_offset_top: px(vv?.offsetTop),
    scroll_y: px(globalThis.scrollY),
    kb_inset: rootStyle.getPropertyValue("--kb-inset").trim() || "unset",
    sheet_top: px(rect?.top),
    sheet_bottom: px(rect?.bottom),
    sheet_in_root: sheet ? root?.contains(sheet) === true : false,
    active_tag: active?.tagName ?? "none",
    active_in_sheet: sheet && active ? sheet.contains(active) : false,
  };
}

/** Sample the keyboard geometry while `name`'s sheet opens and after it
 *  closes. Touch surfaces only; bounded per page load. */
export function useSheetKeyboardDiagnostics(
  name: string,
  enabled: boolean,
  sheet: () => Element | null,
): void {
  useEffect(() => {
    if (!enabled) return undefined;
    const timers = SAMPLE_DELAYS_MS.map((delay) =>
      globalThis.setTimeout(() => {
        if (reportsLeft-- <= 0) return;
        reportClientLog(
          "info",
          "sheet_keyboard_geometry",
          "Compact sheet keyboard geometry",
          {
            sheet: name,
            ...sheetKeyboardGeometry(`open+${String(delay)}`, sheet()),
          },
        );
      }, delay)
    );
    return () => {
      for (const timer of timers) globalThis.clearTimeout(timer);
      globalThis.setTimeout(() => {
        if (reportsLeft-- <= 0) return;
        reportClientLog(
          "info",
          "sheet_keyboard_geometry",
          "Compact sheet keyboard geometry",
          { sheet: name, ...sheetKeyboardGeometry("closed+900", null) },
        );
      }, 900);
    };
  }, [enabled, name]);
}
