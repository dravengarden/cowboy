// Haptic feedback for cowboy — a thin adapter over the shared primitive in
// @shared-utils/ui (./_shell). The shared `haptic()` is native-first: it calls the
// Tauri haptics plugin (real UIImpactFeedbackGenerator on iOS, the ONLY reliable
// iOS haptic) via the injected IPC bridge, and degrades to navigator.vibrate /
// the iOS switch-trick on the plain web/PWA.
//
// History: this file used to look for a `window.__cowboyHaptic` init-script bridge
// that was never actually wired into the native shell, so every call here was a
// silent no-op on iOS. The shell registers `tauri-plugin-haptics` and the shared
// primitive invokes it directly — so delegating here makes ALL existing call sites
// (force-push, jump-to-front, judge long-press) finally buzz on iOS, and any new
// call site is pure web (next deploy, no app reinstall).

import {
  haptic as fireImpact,
  type HapticStyle,
  notificationHaptic,
} from "./_shell";

/** Fire a short impact haptic. The optional `ms` is kept for back-compat with the
 *  existing call sites and mapped to an impact style: ≤12 light, ≤24 medium, else
 *  heavy. Call from within (or just after) a user gesture. */
export function haptic(ms = 12): void {
  const style: HapticStyle = ms <= 12 ? "light" : ms <= 24 ? "medium" : "heavy";
  fireImpact(style);
}

/** Fire a notification haptic for an async OUTCOME — a turn finished (success),
 *  needs you (warning), or errored (error). The distinct iOS pattern reads as
 *  "something resolved", which a plain impact tap can't convey. (Named
 *  `notifyHaptic`, not `notify`, to avoid clashing with the store's snackbar
 *  `notify`.) */
export function notifyHaptic(type: "success" | "warning" | "error"): void {
  notificationHaptic(type);
}
