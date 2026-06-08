import { persisted, useStore } from "./_store/mod.ts";

// Attention alert: a short chime (+ a vibration on devices that support it)
// when an agent needs the user — a finished turn (done) or a permission request
// (confirm). NOT fired for any other event. Toggleable in Settings, DEFAULT ON.
// The setting is persisted + reactive across the app (the Settings switch writes
// it; the store reads it before firing) via @shared-utils/store.
//
// iOS note: the Vibration API is Android-only; iOS Safari/WKWebView has no web
// vibration, so the buzz is a silent no-op there while the chime still plays.

// Default ON: stored as the legacy "1"/"0" string (format preserved) where only
// an explicit "0" disables it — an unset key reads as enabled.
const notify = persisted("cowboy:notify", true, {
  serialize: (on) => (on ? "1" : "0"),
  deserialize: (s) => s !== "0",
});

export function useNotifySetting(): boolean {
  return useStore(notify);
}

export function setNotifySetting(on: boolean): void {
  notify.set(on);
}

// --- Sound (Web Audio) ------------------------------------------------------
//
// A synthesized chime (no asset to ship/decode).
//
// MUST NOT HOLD the audio session: iOS gives Web Audio the non-mixable
// `playback` category, so a RUNNING AudioContext interrupts the user's
// background audio (music / a podcast / liveview's audiobook) for as long as
// it's running — not just while it makes sound. The old code resumed the
// context on every tap and left it running the whole time cowboy was open, so
// merely entering + tapping killed background audio (the reported bug).
//
// Instead we keep the context SUSPENDED whenever it isn't actively chiming:
//   - iOS still needs ONE user gesture to bless the context, so we resume it on
//     the first interaction and immediately suspend it again (a silent resume→
//     suspend doesn't audibly duck anything). After that, programmatic resume()
//     at turn-end is allowed.
//   - playChime resumes, plays the ~0.4s chime, then suspends again — so the
//     only time we touch the session is a brief duck while the chime sounds
//     (exactly how a notification ding behaves), and background audio resumes.

let ctx: AudioContext | undefined;
let unlocked = false;
let suspendTimer: ReturnType<typeof setTimeout> | undefined;

function audioCtor(): typeof AudioContext | undefined {
  const g = globalThis as typeof globalThis & { webkitAudioContext?: typeof AudioContext };
  return g.AudioContext ?? g.webkitAudioContext;
}

function ensureCtx(): AudioContext | undefined {
  if (!ctx) {
    const Ctor = audioCtor();
    if (!Ctor) return undefined;
    ctx = new Ctor();
  }
  return ctx;
}

// Release the audio session shortly after a chime so background audio resumes.
// Debounced so back-to-back chimes don't suspend mid-sound.
function scheduleSuspend(c: AudioContext): void {
  if (suspendTimer) clearTimeout(suspendTimer);
  suspendTimer = setTimeout(() => {
    suspendTimer = undefined;
    if (c.state === "running") void c.suspend();
  }, 600);
}

// Bless the context within a user gesture ONCE (iOS requires a gesture for the
// first resume), then immediately suspend — so we never keep the session active
// just from being open. Only if the chime is enabled; otherwise we never touch
// audio at all.
function unlockOnce(): void {
  if (unlocked || !notify.get()) return;
  const c = ensureCtx();
  if (!c) return;
  unlocked = true;
  void c.resume().then(() => {
    if (c.state === "running") void c.suspend();
  });
}
if (typeof globalThis.addEventListener === "function") {
  for (const ev of ["pointerdown", "keydown", "touchend"]) {
    globalThis.addEventListener(ev, unlockOnce, { passive: true });
  }
}

function emitChime(c: AudioContext): void {
  const now = c.currentTime;
  // A gentle two-note rise (A5 → E6), each ~0.18s with a quick attack + decay.
  for (const [i, freq] of [880, 1318.5].entries()) {
    const osc = c.createOscillator();
    const gain = c.createGain();
    osc.type = "sine";
    osc.frequency.value = freq;
    const start = now + i * 0.1;
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.18, start + 0.015);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.18);
    osc.connect(gain).connect(c.destination);
    osc.start(start);
    osc.stop(start + 0.2);
  }
  scheduleSuspend(c);
}

function playChime(): void {
  const c = ensureCtx();
  if (!c) return;
  // Resume only to sound the chime; emit AFTER resume resolves so currentTime is
  // valid, then suspend again (scheduleSuspend) to release the session.
  if (c.state === "suspended") {
    void c.resume().then(() => emitChime(c));
  } else {
    emitChime(c);
  }
}

/**
 * Fire the attention alert if enabled: play the chime and, where supported, a
 * short vibration. Called from the store ONLY for the two events that need the
 * user — a finished turn (done) and a permission request (needs confirmation) —
 * never for mid-turn churn.
 */
export function fireAlert(): void {
  if (!notify.get()) return;
  // Only alert when the user ISN'T actively looking. A turn that finishes while
  // you're watching (e.g. the agent's quick reply right after you hit send) you
  // can already SEE — chiming then is just noise. The alert is for when you've
  // switched away; the tab being hidden is exactly that signal.
  if (globalThis.document?.visibilityState === "visible") return;
  playChime();
  // Android only; iOS has no web Vibration API (silent no-op there).
  if (typeof globalThis.navigator?.vibrate === "function") {
    try {
      globalThis.navigator.vibrate(30);
    } catch {
      /* unsupported / blocked — ignore */
    }
  }
}
