import { useSyncExternalStore } from "react";

// Where the navbar (AppBar) sits on the mobile/compact tier. "top" is the
// classic placement; "bottom" makes it a mobile-browser-style bottom bar (the
// composer stays at the very bottom, the bar floats just above it). The setting
// is mobile-only — desktop is always "top" — and it's applied by App, which
// gates on the same `breakpoints.down("lg")` tier that drives the rest of the
// mobile shell. Persisted in localStorage, reactive across tabs, same
// useSyncExternalStore pattern as readingSettings / vimSetting.

export type NavbarPosition = "top" | "bottom";

const KEY = "cowboy:navbar-pos";
const EVENT = "cowboy:navbar-pos-change";
const DEFAULT: NavbarPosition = "top";

function read(): NavbarPosition {
  return globalThis.localStorage?.getItem(KEY) === "bottom" ? "bottom" : "top";
}

// The snapshot is a primitive, so it's referentially stable by value — no
// caching object is needed (unlike readingSettings' object snapshot).
let snapshot: NavbarPosition = read();

function subscribe(onChange: () => void): () => void {
  const handler = (): void => {
    snapshot = read();
    onChange();
  };
  globalThis.addEventListener?.(EVENT, handler);
  globalThis.addEventListener?.("storage", handler); // other tabs
  return () => {
    globalThis.removeEventListener?.(EVENT, handler);
    globalThis.removeEventListener?.("storage", handler);
  };
}

export function useNavbarPosition(): NavbarPosition {
  return useSyncExternalStore(
    subscribe,
    () => snapshot,
    () => DEFAULT,
  );
}

export function setNavbarPosition(pos: NavbarPosition): void {
  globalThis.localStorage?.setItem(KEY, pos);
  // dispatchEvent runs listeners synchronously, so `snapshot` is refreshed
  // before this returns — the caller's next render sees the new value.
  globalThis.dispatchEvent?.(new Event(EVENT));
}
