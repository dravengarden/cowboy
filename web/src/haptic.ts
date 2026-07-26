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
import { hapticStyleForIntent } from "./hapticIntent";
export {
  type CowboyHapticIntent,
  type CowboyHapticStyle,
  hapticStyleForIntent,
} from "./hapticIntent";

/**
 * Cowboy's product-level haptic hierarchy. Features choose meaning, never an
 * arbitrary duration:
 * - navigation: lightweight movement, selection, disclosure
 * - magnetic: a spatial target or sticky boundary has engaged
 * - confirmation: a deliberate, meaningful but non-destructive commitment
 * - important: a high-consequence destructive/interruption confirmation
 *
 * Async outcomes use `notifyHaptic` below because their patterned feedback is
 * semantically different from impact strength.
 */
export function navigationHaptic(): void {
  fireImpact(hapticStyleForIntent("navigation"));
}

export function magneticHaptic(): void {
  fireImpact(hapticStyleForIntent("magnetic"));
}

export function confirmationHaptic(): void {
  fireImpact(hapticStyleForIntent("confirmation"));
}

export function importantHaptic(): void {
  fireImpact(hapticStyleForIntent("important"));
}

/** Backward-compatible adapter for ordinary call sites awaiting semantic
 * migration. New feature code must use one of the intent functions above. */
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
