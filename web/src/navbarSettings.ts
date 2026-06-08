import { useMediaQuery, useTheme } from "@mui/material";
import { persisted, useStore } from "./_store/mod.ts";

// Where the navbar (AppBar) sits on the mobile/compact tier. "top" is the
// classic placement; "bottom" makes it a mobile-browser-style bottom bar (the
// composer stays at the very bottom, the bar floats just above it). The setting
// is mobile-only — desktop is always "top" — and it's applied by App, which
// gates on the same `breakpoints.down("lg")` tier that drives the rest of the
// mobile shell. Persisted + reactive across tabs via @shared-utils/store.

export type NavbarPosition = "top" | "bottom";

// Stored as the raw "top"/"bottom" string (legacy format preserved); an
// unset/garbage value falls back to "bottom" (the product default).
const navbar = persisted<NavbarPosition>("cowboy:navbar-pos", "bottom", {
  serialize: (p) => p,
  deserialize: (s) => (s === "top" || s === "bottom" ? s : "bottom"),
});

export function useNavbarPosition(): NavbarPosition {
  return useStore(navbar);
}

export function setNavbarPosition(pos: NavbarPosition): void {
  navbar.set(pos);
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
