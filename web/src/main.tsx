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

// Size the app to the *visual* viewport height (the area not covered by the iOS
// software keyboard), published as the `--app-height` CSS var the #root rule
// reads (see index.html). On iOS Safari the layout viewport doesn't shrink when
// the keyboard opens (only the visual viewport does), so a full-height column
// otherwise leaves the bottom composer behind the keyboard. We track height
// ONLY: a previous attempt also followed visualViewport.offsetTop via a
// transform, but in Safari that over-lifted the app. No-op on desktop (visual
// == layout viewport). Listeners are passive and never removed — the app lives
// for the whole document lifetime.
function syncAppHeight(): void {
  const vv = globalThis.visualViewport;
  document.documentElement.style.setProperty(
    "--app-height",
    `${vv ? vv.height : globalThis.innerHeight}px`,
  );
}
const vv = globalThis.visualViewport;
if (vv) {
  vv.addEventListener("resize", syncAppHeight);
  vv.addEventListener("scroll", syncAppHeight);
}
globalThis.addEventListener("orientationchange", syncAppHeight);
syncAppHeight();

// Diagnostic overlay, opt-in via `?vvdebug=1`. Prints the live viewport numbers
// so the iOS keyboard behaviour can be read off-device. Off (and tree-shaken to
// a dead branch) for everyone else. Remove once the keyboard handling is final.
if (new URLSearchParams(globalThis.location.search).has("vvdebug")) {
  const dbg = document.createElement("div");
  dbg.style.cssText =
    "position:fixed;top:0;left:0;right:0;z-index:99999;background:rgba(0,0,0,.82);" +
    "color:#0f0;font:11px/1.5 ui-monospace,monospace;padding:3px 6px;" +
    "pointer-events:none;white-space:pre-wrap;";
  const render = (): void => {
    const v = globalThis.visualViewport;
    const appH = getComputedStyle(document.documentElement)
      .getPropertyValue("--app-height")
      .trim();
    dbg.textContent =
      `inner=${globalThis.innerHeight} ` +
      `vv.h=${v ? Math.round(v.height) : "-"} ` +
      `vv.top=${v ? Math.round(v.offsetTop) : "-"} ` +
      `--app-height=${appH || "(unset)"}`;
  };
  document.addEventListener("DOMContentLoaded", () => document.body.appendChild(dbg));
  if (document.body) document.body.appendChild(dbg);
  vv?.addEventListener("resize", render);
  vv?.addEventListener("scroll", render);
  globalThis.addEventListener("resize", render);
  globalThis.setInterval(render, 250);
  render();
}

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <Root />
    </StrictMode>,
  );
}
