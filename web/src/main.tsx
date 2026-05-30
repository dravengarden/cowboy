import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { App } from "./App";
import { useThemeMode } from "./theme";
import { PortalProvider } from "./_shell";

function Root(): React.JSX.Element {
  const { theme, mode, setMode } = useThemeMode();
  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <PortalProvider appId="cowboy">
        <App themeMode={mode} onSetThemeMode={setMode} />
      </PortalProvider>
    </ThemeProvider>
  );
}

// Pin the app to the *visual* viewport (the area not covered by the iOS
// software keyboard), published as the `--app-height` / `--app-offset-top` CSS
// vars the #root rule reads (see index.html). On iOS Safari the layout viewport
// doesn't shrink when the keyboard opens; sometimes iOS resizes the visual
// viewport (height shrinks) and sometimes it scrolls it (offsetTop grows to
// reveal the focused input). Publishing BOTH lets #root (position:fixed +
// translateY) follow the visible rectangle either way, so the bottom composer
// always sits above the keyboard without relying on iOS's flaky
// scroll-into-view. No-op on desktop (offsetTop 0, height == layout viewport).
// Listeners are passive and never removed — the app lives for the whole
// document lifetime.
function syncAppHeight(): void {
  const vv = globalThis.visualViewport;
  const root = document.documentElement;
  root.style.setProperty("--app-height", `${vv ? vv.height : globalThis.innerHeight}px`);
  root.style.setProperty("--app-offset-top", `${vv ? vv.offsetTop : 0}px`);
}
const vv = globalThis.visualViewport;
if (vv) {
  vv.addEventListener("resize", syncAppHeight);
  vv.addEventListener("scroll", syncAppHeight);
}
globalThis.addEventListener("orientationchange", syncAppHeight);
syncAppHeight();

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <Root />
    </StrictMode>,
  );
}
