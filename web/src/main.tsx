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

// Keep the app sized to the *visual* viewport (the area not covered by the iOS
// software keyboard), published as the `--app-height` CSS var the root height
// reads (see index.html). On iOS Safari the layout viewport doesn't shrink when
// the keyboard opens, so a full-height non-scrolling column can't scroll the
// focused input above the keyboard — intermittently hiding it. Shrinking the
// column to the visible area keeps the bottom composer above the keyboard
// without relying on iOS's flaky scroll-into-view. No-op on desktop (visual ==
// layout viewport). Listeners are passive and never removed — the app lives for
// the whole document lifetime.
function syncAppHeight(): void {
  const vv = globalThis.visualViewport;
  const height = vv ? vv.height : globalThis.innerHeight;
  document.documentElement.style.setProperty("--app-height", `${height}px`);
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
