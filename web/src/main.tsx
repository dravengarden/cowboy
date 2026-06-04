import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { App } from "./App";
import { useThemeMode } from "./theme";

function Root(): React.JSX.Element {
  const { theme, mode, setMode } = useThemeMode();
  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <App themeMode={mode} onSetThemeMode={setMode} />
    </ThemeProvider>
  );
}

// iOS keyboard handling lives in CSS now (see index.html): #root is 100dvh and
// the body is locked, with interactive-widget=resizes-content in the viewport
// meta. The browser shrinks the layout viewport — and so #root — when the
// keyboard opens, with no JS. The previous visualViewport --app-height scheme
// is gone: on a real iPhone visualViewport.height is smaller than the
// position:fixed body, so sizing #root from it left a blank strip at the bottom
// (invisible in Chrome's device emulator, which doesn't split visual vs layout
// viewport). Matches the atlantis portal, which uses the same dvh approach.

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <Root />
    </StrictMode>,
  );
}

// Standalone PWA: register the service worker (offline shell + installable).
if (import.meta.env.PROD && "serviceWorker" in navigator) {
  window.addEventListener("load", () => {
    void navigator.serviceWorker.register("/sw.js").catch(() => {});
  });
}
