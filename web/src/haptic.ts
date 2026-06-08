// Best-effort haptic feedback for a deliberate, weighty action (force-push).
//
// Platform reality (the iOS catch):
//  - Android (Chrome/Firefox): the Vibration API works → navigator.vibrate.
//  - iOS Safari / WKWebView (incl. a thin Tauri shell): Apple never shipped the
//    Vibration API, so navigator.vibrate is a SILENT no-op. The only web haptic on
//    iOS is the `<input type="checkbox" switch>` "label" trick (iOS 17.4+):
//    toggling such a switch within a user-activation window emits a light tap.
//    It's best-effort + fragile; a native Tauri haptics plugin is the reliable
//    iOS route if/when the app wires one.
//
// Call from within (or shortly after) a user gesture so iOS user-activation is
// still live. Both paths are attempted; only the supported one does anything, so
// at most one tap fires per platform.

let switchLabel: HTMLLabelElement | undefined;

// iOS 17.4+ light tap via a hidden switch toggle. No-op (harmless) elsewhere.
function iosSwitchTap(): void {
  const doc = globalThis.document as Document | undefined;
  if (doc?.body === undefined || doc.body === null) {
    return;
  }
  if (switchLabel === undefined) {
    const label = doc.createElement("label");
    label.setAttribute("aria-hidden", "true");
    label.style.cssText =
      "position:fixed;top:0;left:0;width:0;height:0;opacity:0;pointer-events:none;overflow:hidden";
    const input = doc.createElement("input");
    input.type = "checkbox";
    // The `switch` attribute is what makes iOS emit the haptic on toggle.
    input.setAttribute("switch", "");
    label.append(input);
    doc.body.append(label);
    switchLabel = label;
  }
  switchLabel.querySelector("input")?.click();
}

/** Fire a short, light haptic tap where the platform supports one. */
export function haptic(ms = 12): void {
  try {
    const nav = globalThis.navigator;
    if (typeof nav?.vibrate === "function") {
      nav.vibrate(ms);
    }
  } catch {
    // unsupported — ignore
  }
  try {
    iosSwitchTap();
  } catch {
    // unsupported — ignore
  }
}
