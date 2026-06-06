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
// A synthesized chime (no asset to ship/decode). iOS + Safari refuse to start
// audio until a user gesture, so we lazily create one shared AudioContext and
// resume() it on the first interaction; once unlocked, programmatic plays work
// for the rest of the session (the user has always tapped/typed long before a
// turn ends).

let ctx: AudioContext | undefined;

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

function unlock(): void {
  const c = ensureCtx();
  if (c && c.state === "suspended") void c.resume();
}

// Resume the context on any user gesture (cheap; handles re-suspension after the
// PWA is backgrounded too). Registered once at module load, passive.
if (typeof globalThis.addEventListener === "function") {
  for (const ev of ["pointerdown", "keydown", "touchend"]) {
    globalThis.addEventListener(ev, unlock, { passive: true });
  }
}

function playChime(): void {
  const c = ensureCtx();
  if (!c) return;
  if (c.state === "suspended") void c.resume();
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
