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

// The three things worth a sound, each with a SEMANTICALLY-shaped chime:
//  - done    : a bright two-note rise that RESOLVES up (A5→E6) — "finished".
//  - decision: a softer, open two-note rise of a fourth (G5→C6) that doesn't fully
//              resolve — reads as a question / "your turn".
//  - error   : a LOW descending pair on a triangle wave (A4→D#4) — darker + buzzier,
//              unmistakably "something went wrong".
export type AlertKind = "done" | "decision" | "error";

interface Note {
  freq: number;
  type: OscillatorType;
  at: number; // start offset (s)
  dur: number; // decay length (s)
  peak: number; // gain peak
}

const NOTES: Record<AlertKind, readonly Note[]> = {
  done: [
    { freq: 880, type: "sine", at: 0, dur: 0.18, peak: 0.18 },
    { freq: 1318.5, type: "sine", at: 0.1, dur: 0.18, peak: 0.18 },
  ],
  decision: [
    { freq: 783.99, type: "sine", at: 0, dur: 0.16, peak: 0.15 },
    { freq: 1046.5, type: "sine", at: 0.12, dur: 0.24, peak: 0.15 },
  ],
  error: [
    { freq: 440, type: "triangle", at: 0, dur: 0.22, peak: 0.14 },
    { freq: 311.13, type: "triangle", at: 0.15, dur: 0.34, peak: 0.14 },
  ],
};

// Vibration rhythm per kind (Android only): a single tap for done, a double "ask"
// for decision, a longer urgent pattern for error.
const BUZZ: Record<AlertKind, number | number[]> = {
  done: 30,
  decision: [20, 60, 20],
  error: [55, 40, 55],
};

function emitChime(c: AudioContext, kind: AlertKind): void {
  const now = c.currentTime;
  for (const n of NOTES[kind]) {
    const osc = c.createOscillator();
    const gain = c.createGain();
    osc.type = n.type;
    osc.frequency.value = n.freq;
    const start = now + n.at;
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(n.peak, start + 0.015);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + n.dur);
    osc.connect(gain).connect(c.destination);
    osc.start(start);
    osc.stop(start + n.dur + 0.02);
  }
  scheduleSuspend(c);
}

function playChime(kind: AlertKind): void {
  const c = ensureCtx();
  if (!c) return;
  // Resume only to sound the chime; emit AFTER resume resolves so currentTime is
  // valid, then suspend again (scheduleSuspend) to release the session.
  if (c.state === "suspended") {
    void c.resume().then(() => emitChime(c, kind));
  } else {
    emitChime(c, kind);
  }
}

/**
 * Fire the attention alert if enabled: play the kind's chime + (Android) its
 * vibration. Called from the store ONLY for the three things that need the user:
 * a finished task (`done`), a point where they must decide (`decision` — the judge
 * saw a question, or a permission request), and an agent problem (`error`). NOT
 * for mid-turn churn, a plain continue, or a force-push (those produce no verdict).
 */
export function fireAlert(kind: AlertKind): void {
  if (!notify.get()) return;
  // Only alert when the user ISN'T actively looking. A turn that finishes while
  // you're watching (e.g. the agent's quick reply right after you hit send) you
  // can already SEE — chiming then is just noise. The alert is for when you've
  // switched away; the tab being hidden is exactly that signal.
  if (globalThis.document?.visibilityState === "visible") return;
  playChime(kind);
  // Android only; iOS has no web Vibration API (silent no-op there).
  if (typeof globalThis.navigator?.vibrate === "function") {
    try {
      globalThis.navigator.vibrate(BUZZ[kind]);
    } catch {
      /* unsupported / blocked — ignore */
    }
  }
}
