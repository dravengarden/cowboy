// When a deployed build may replace the running one.
//
// One rule set for every Cowboy surface (docs/offline-first-sync.md §Update
// policy): a redeploy is applied by the client itself, never by a button the
// user has to find. The rules exist so that "by itself" never means "while the
// user was in the middle of something". Kept pure and DOM-free so the policy is
// testable on its own; `useAutoUpdate` owns the timers and the actual reload.

/** What the surface knows about its user at one instant. */
export interface UpdateGate {
  /** The app reports nothing a reload would destroy: no composer text or
   *  attachments, no in-flight write, no running turn, no focused editor. */
  readonly idle: boolean;
  /** The page is in the foreground. */
  readonly visible: boolean;
  /** How long it has been continuously foreground. A resumed page starts over:
   *  an installed PWA restores a frozen page, and reloading it in the second
   *  after someone opened the app reads as a crash, not as an update. */
  readonly visibleForMs: number;
}

export interface UpdateCountdown {
  /** Seconds left before the update is applied. */
  readonly secs: number;
  /** The countdown is parked because the gate is shut. */
  readonly held: boolean;
}

/** Whether the running build may be replaced right now. `minVisibleMs` is the
 *  foreground dwell the surface requires; 0 asks for no dwell at all. */
export function updateAllowed(gate: UpdateGate, minVisibleMs: number): boolean {
  if (!gate.idle) return false;
  if (minVisibleMs <= 0) return true;
  return gate.visible && gate.visibleForMs >= minVisibleMs;
}

/** One second of the countdown. A shut gate rewinds to the start instead of
 *  freezing mid-count, so a reload is always preceded by a whole countdown the
 *  user could see. */
export function tickUpdateCountdown(
  current: UpdateCountdown,
  allowed: boolean,
  countdownSecs: number,
): UpdateCountdown {
  if (!allowed) return { secs: countdownSecs, held: true };
  return { secs: current.secs - 1, held: false };
}
