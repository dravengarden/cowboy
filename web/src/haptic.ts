// Haptic feedback for cowboy — a thin adapter over the app-shell component.
// The shared `haptic()` is native-first: it calls the
// Tauri haptics plugin (real UIImpactFeedbackGenerator on iOS, the ONLY reliable
// iOS haptic) via the injected IPC bridge, and degrades to navigator.vibrate /
// the iOS switch-trick on the plain web/PWA.
//
// History: this file used to look for a `window.__cowboyHaptic` init-script bridge
// that was never actually wired into the native shell, so every call here was a
// silent no-op on iOS. The shell registers `tauri-plugin-haptics` and the shared
// primitive invokes it directly — so delegating here makes ALL existing call sites
// (force-push and jump-to-front) finally buzz on iOS, and any new
// call site is pure web (next deploy, no app reinstall).

import {
  haptic as fireImpact,
  type HapticStyle,
  notificationHaptic,
  selectionHaptic,
} from "@cowboy/app-shell";
import { hapticStyleForIntent } from "./hapticIntent";
export {
  type CowboyHapticIntent,
  type CowboyHapticStyle,
  hapticStyleForIntent,
} from "./hapticIntent";
import type { CowboyHapticIntent } from "./hapticIntent";

interface CowboyNativeSelectionBridge {
  readonly __cowboyPrepareSelectionHaptic?: () => void;
  readonly __cowboySelectionHaptic?: () => void;
}

/** Warm the native low-priority soft-impact generator before a drag can cross
 * its magnetic threshold. The bridge names remain stable for older bundles. */
export function prepareNavigationHaptic(): void {
  const bridge = globalThis as typeof globalThis & CowboyNativeSelectionBridge;
  if (typeof bridge.__cowboyPrepareSelectionHaptic !== "function") return;
  bridge.__cowboyPrepareSelectionHaptic();
}

function fireNativeLowPriorityHaptic(): boolean {
  const bridge = globalThis as typeof globalThis & CowboyNativeSelectionBridge;
  if (typeof bridge.__cowboySelectionHaptic !== "function") return false;
  bridge.__cowboySelectionHaptic();
  return true;
}

function fireIntent(intent: CowboyHapticIntent): void {
  const style = hapticStyleForIntent(intent);
  if (style === "selection") {
    if (fireNativeLowPriorityHaptic()) return;
    selectionHaptic();
    return;
  }
  fireImpact(style);
}

/**
 * Cowboy's product-level haptic hierarchy. Features choose meaning, never an
 * arbitrary duration. Strength follows danger / consequence:
 * - navigation: browse, disclose, pick a session, open a tool card
 * - magnetic: a spatial target or sticky boundary has engaged
 * - confirmation: a constructive commit (send, schedule, apply)
 * - important: a destructive or interrupting confirmation
 *
 * Async outcomes use `notifyHaptic` below because their patterned feedback is
 * semantically different from impact strength.
 */
export function navigationHaptic(): void {
  fireIntent("navigation");
}

export function magneticHaptic(): void {
  fireIntent("magnetic");
}

export function confirmationHaptic(): void {
  fireIntent("confirmation");
}

export function importantHaptic(): void {
  fireIntent("important");
}

/** Backward-compatible adapter for ordinary call sites awaiting semantic
 * migration. New feature code must use one of the intent functions above. */
export function haptic(ms = 12): void {
  if (ms <= 12 && fireNativeLowPriorityHaptic()) return;
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
