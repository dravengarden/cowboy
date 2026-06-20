import { useMediaQuery, useTheme } from "@mui/material";

// Where the navbar (AppBar) sits. On the compact (mobile/tablet) tier it's
// ALWAYS at the bottom — the mobile-browser-style bottom bar (the composer stays
// at the very bottom, the bar floats just above it) — and on desktop it's a top
// bar. There's no longer a user choice for this: cowboy committed to a bottom
// bar on mobile, so this is just "are we on the compact tier". It's the single
// source of truth shared by App (which moves the AppBar) and every modal (which
// passes it as BottomSheet `forceSheet`, so modals keep rising from the bottom
// on a tablet too instead of switching to a centered dialog).

export function useNavbarAtBottom(): boolean {
  const theme = useTheme();
  return useMediaQuery(theme.breakpoints.down("lg"));
}
