// Did the build we just swapped into actually start?
//
// The update path can only promise so much. It proves the new build is wholly
// cached before it swaps, so the network can no longer strand anyone — but a
// build that downloads perfectly can still fail to run, and a PWA that boots
// into a broken build is bricked until the next deploy. The device already
// holds the answer: the service worker keeps two generations, so the build the
// user was running moments ago is still whole in cache.
//
// What is missing is a way to know. A crash during boot looks exactly like a
// slow boot, and a user who kills a white-screened app leaves no error behind.
// So the swap writes down that it happened, and the build that follows has to
// sign for it:
//
//   swapping  the old build wrote this immediately before reloading
//   booting   the new build's document ran; it has not finished starting
//   (absent)  a build started and stayed up
//
// A load that finds `booting` is therefore the load AFTER one that reached the
// document and never came up — the one signature a white screen leaves. The
// phases are written by an inline script in `index.html`, before the first
// module, because the failure being watched for includes "the entry chunk
// never evaluated". This module owns the same strings and the rules over them;
// `updateAttempt.test.ts` pins the two copies together.

export const UPDATE_ATTEMPT_KEY = "cowboy:update-attempt";

/** How long a written attempt stays meaningful. Past this, a stale marker left
 *  by a device that was closed mid-update is not evidence of anything. */
export const UPDATE_ATTEMPT_TTL_MS = 30 * 60_000;

export interface UpdateAttempt {
  /** The build being swapped to, for the notice and the logs. */
  readonly to: string;
  readonly at: number;
  readonly phase: "swapping" | "booting";
}

export interface AttemptStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

function readAttempt(raw: string | null): UpdateAttempt | undefined {
  if (raw === null) return undefined;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return undefined;
    const { to, at, phase } = parsed as Partial<UpdateAttempt>;
    if (typeof at !== "number" || !Number.isFinite(at)) return undefined;
    if (phase !== "swapping" && phase !== "booting") return undefined;
    return { to: typeof to === "string" ? to : "unknown", at, phase };
  } catch {
    return undefined;
  }
}

/** What this load should do about whatever the last one left behind.
 *
 *  `failed` is set only for the one case that means a build did not start: a
 *  document that ran, wrote `booting`, and was never followed by a build that
 *  signed off. */
export interface AttemptTransition {
  readonly next: UpdateAttempt | undefined;
  readonly failed: UpdateAttempt | undefined;
}

export function advanceUpdateAttempt(
  raw: string | null,
  now: number,
): AttemptTransition {
  const attempt = readAttempt(raw);
  if (!attempt) return { next: undefined, failed: undefined };
  if (now - attempt.at > UPDATE_ATTEMPT_TTL_MS || now < attempt.at) {
    return { next: undefined, failed: undefined };
  }
  if (attempt.phase === "swapping") {
    // This is the load the swap was made for. Sign in, then start.
    return { next: { ...attempt, phase: "booting" }, failed: undefined };
  }
  return { next: undefined, failed: attempt };
}

/** Record that the page is about to be replaced by `to`. */
export function markUpdateSwapping(
  storage: AttemptStorage | undefined,
  to: string,
  now: number,
): void {
  try {
    storage?.setItem(UPDATE_ATTEMPT_KEY, JSON.stringify({ to, at: now, phase: "swapping" }));
  } catch {
    // A device that denies storage simply loses the rollback guard; it must
    // never lose the update.
  }
}

/** Sign for the swap: this build started and stayed up. */
export function clearUpdateAttempt(storage: AttemptStorage | undefined): void {
  try {
    storage?.removeItem(UPDATE_ATTEMPT_KEY);
  } catch {
    // See above.
  }
}

/** Whether a swap is still unsigned, so a crash now belongs to it. */
export function updateSwapInFlight(storage: AttemptStorage | undefined): boolean {
  try {
    return readAttempt(storage?.getItem(UPDATE_ATTEMPT_KEY) ?? null) !== undefined;
  } catch {
    return false;
  }
}

/** Where to go once the previous build is back in the shell cache.
 *
 *  Not `location.reload()`: WKWebView can replay the stale document it already
 *  has, which is the one that just failed (see moduleRecovery). A distinct URL
 *  forces a real navigation, and the service worker answers it from cache.
 *
 *  Deliberately NOT one of the recovery params either. Those are network-first
 *  by design, and the network holds exactly the build we are running away
 *  from — asking for it again is the one thing this must never do. */
export function rolledBackNavigationUrl(currentUrl: string, now: number): string {
  const target = new URL(currentUrl);
  target.searchParams.set("cowboy-rolled-back", String(now));
  return target.toString();
}

export interface RollbackResult {
  readonly ok: boolean;
  readonly version?: string;
  readonly attempts?: number;
}

const ROLLBACK_TIMEOUT_MS = 5_000;

/** Ask the service worker to put back the last build that started.
 *
 *  Deliberately standalone: this runs before the app exists, on a page whose
 *  build may be the broken one, so it must not pull in the store, the theme or
 *  anything else that could be what failed. */
export async function rollbackToPreviousBuild(): Promise<RollbackResult> {
  const controller = globalThis.navigator?.serviceWorker?.controller;
  if (!controller || typeof MessageChannel === "undefined") return { ok: false };
  return await new Promise<RollbackResult>((resolve) => {
    const channel = new MessageChannel();
    const timer = setTimeout(() => resolve({ ok: false }), ROLLBACK_TIMEOUT_MS);
    channel.port1.onmessage = (event: MessageEvent<RollbackResult>): void => {
      clearTimeout(timer);
      resolve(event.data ?? { ok: false });
    };
    try {
      controller.postMessage({ type: "cowboy.rollback-shell" }, [channel.port2]);
    } catch {
      clearTimeout(timer);
      resolve({ ok: false });
    }
  });
}
