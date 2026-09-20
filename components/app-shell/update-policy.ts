// When a deployed build may replace the running one.
//
// One rule set for every Cowboy surface (docs/offline-first-sync.md §Update
// policy): a redeploy is applied by the client itself, never by a button the
// user has to find. The rules exist so that "by itself" never means "while the
// user was in the middle of something". A surface may also offer to bring that
// reload forward, but the offer is an accelerator — nothing waits on it. Kept
// pure and DOM-free so the policy is testable on its own; `useAutoUpdate` owns
// the timers, the download and the actual reload.

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

/** Where a pending update is. `downloading` until its bits are cached, `ready`
 *  while they wait for the swap, `reloading` during it, `failed` when they did
 *  not all arrive and this build keeps running.
 *
 *  The last two are what a rollback leaves behind: `rejected` once this device
 *  has watched the deployed build fail to start and put back the one that did,
 *  and `abandoned` once that has happened twice — at which point the deploy is
 *  broken rather than unlucky, there is nothing useful left to offer, and the
 *  surface says nothing at all until a different build is deployed. */
export type UpdatePhase =
  | "downloading"
  | "ready"
  | "reloading"
  | "failed"
  | "rejected"
  | "abandoned";

/** Everything that decides whether the running build is replaced this instant. */
export interface UpdateIntent {
  /** The deployed build is wholly cached, so the swap needs no network. Nothing
   *  replaces a running build before its replacement is here. */
  readonly downloaded: boolean;
  /** The user pressed the update control. Consent outranks the idle gate and
   *  the foreground dwell: those exist to protect someone who did not ask, and
   *  this someone is looking at the control they just pressed. */
  readonly requested: boolean;
  /** The automatic path's visible countdown has run out (see `updateAllowed`
   *  and `tickUpdateCountdown`). */
  readonly countedDown: boolean;
}

/** Whether to swap builds right now. The download is the one condition no
 *  intent can waive; past it, either road leads to the same reload. */
export function updateReloadsNow(intent: UpdateIntent): boolean {
  if (!intent.downloaded) return false;
  return intent.requested || intent.countedDown;
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
