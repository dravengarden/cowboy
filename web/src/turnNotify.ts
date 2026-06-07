import { useSyncExternalStore } from "react";

// "Turn complete" alert: a short chime (+ a vibration on devices that support
// it) when an agent finishes a turn. Toggleable in Settings, DEFAULT ON. The
// setting is persisted in localStorage and reactive across the app (the Settings
// switch writes it; the store reads it before firing) — same useSyncExternalStore
// pattern as vimSetting.
//
// iOS note: the Vibration API is Android-only; iOS Safari/WKWebView has no web
// vibration, so the buzz is a silent no-op there while the chime still plays.

const KEY = "cowboy:notify";
const EVENT = "cowboy:notify-change";

// Default ON: only an explicit "0" disables it (an unset key reads as enabled).
function getNotifySetting(): boolean {
  return globalThis.localStorage?.getItem(KEY) !== "0";
}

function subscribe(onChange: () => void): () => void {
  globalThis.addEventListener?.(EVENT, onChange);
  globalThis.addEventListener?.("storage", onChange); // other tabs
  return () => {
    globalThis.removeEventListener?.(EVENT, onChange);
    globalThis.removeEventListener?.("storage", onChange);
  };
}

export function useNotifySetting(): boolean {
  return useSyncExternalStore(subscribe, getNotifySetting, () => true);
}

export function setNotifySetting(on: boolean): void {
  globalThis.localStorage?.setItem(KEY, on ? "1" : "0");
  globalThis.dispatchEvent?.(new Event(EVENT));
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
  if (unlocked || !getNotifySetting()) return;
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
 * Fire the turn-complete alert if enabled: play the chime and, where supported,
 * a short vibration. Called from the store on a session's busy→running edge.
 */
export function fireTurnComplete(): void {
  if (!getNotifySetting()) return;
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
