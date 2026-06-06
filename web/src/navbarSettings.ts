import { useSyncExternalStore } from "react";
import { useMediaQuery, useTheme } from "@mui/material";

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
const DEFAULT: NavbarPosition = "bottom";

function read(): NavbarPosition {
  // An explicit stored "top"/"bottom" wins; an unset/garbage value falls back to
  // DEFAULT (so the product default is honoured, not a hard-coded "top").
  const v = globalThis.localStorage?.getItem(KEY);
  return v === "bottom" || v === "top" ? v : DEFAULT;
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

// True when the navbar is actually rendered at the bottom: the compact tier
// (`< lg`, same as the rest of the mobile shell — tablets included) AND the
// user picked "bottom". The single source of truth shared by App (which moves
// the AppBar) and every modal (which passes it as BottomSheet `forceSheet`, so
// the modals keep rising from the bottom on a tablet too instead of switching
// to a centered dialog). Both hooks run unconditionally, then combine.
export function useNavbarAtBottom(): boolean {
  const theme = useTheme();
  const compact = useMediaQuery(theme.breakpoints.down("lg"));
  const pos = useNavbarPosition();
  return compact && pos === "bottom";
}
