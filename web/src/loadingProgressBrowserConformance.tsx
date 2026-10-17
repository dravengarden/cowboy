import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CircularProgress, CssBaseline, Stack, Typography } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { StatusDot } from "./SessionStatusDot";
import { mobileCompositorFlattenSx } from "./mobilePresentationMotion";

/** Real theme, synthetic activity, no product account. Keep mounted for CDP
 * screenshots and reduced-motion / forced-colors emulation. */
export function runLoadingProgressBrowserConformance(keepFixture = false): string[] {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const results: string[] = [];
  const check = (ok: boolean, label: string) => {
    if (!ok) throw new Error(label);
    results.push(label);
  };
  const originalFont = document.documentElement.style.fontSize;
  const originalUnit = document.documentElement.style.getPropertyValue("--cowboy-desktop-size-unit");
  flushSync(() => root.render(
    <BrowserProductTheme>
      <CssBaseline />
      <Stack spacing={3} sx={{ p: 3, maxWidth: 520 }}>
        <Typography variant="h6">Loading · Cowboy</Typography>
        {(["busy", "starting", "running", "exited"] as const).map((status) => (
          <Stack key={status} direction="row" spacing={1.5} alignItems="center">
            <StatusDot status={status} /><Typography>{status}</Typography>
          </Stack>
        ))}
        <Stack direction="row" spacing={2} alignItems="center">
          {[12, 16, 24, 40].map((size) => <CircularProgress key={size} size={size} aria-label={`Loading ${size}`} />)}
        </Stack>
        <CircularProgress variant="determinate" value={65} size={24} aria-label="Progress" />
        <Box sx={mobileCompositorFlattenSx}>
          <Box data-mobile-drawer-surface>
            <CircularProgress data-session-activity="true" size={16} aria-label="Paused during swipe" />
          </Box>
        </Box>
      </Stack>
    </BrowserProductTheme>,
  ));
  try {
    const busy = container.querySelector<HTMLElement>('[aria-label="Running…"]')!;
    const numeric = container.querySelector<HTMLElement>('[aria-label="Loading 24"]')!;
    const idle = container.querySelector<HTMLElement>('[aria-label="Live"]')!;
    for (const unit of ["16px", "1rem"]) {
      document.documentElement.style.setProperty("--cowboy-desktop-size-unit", unit);
      document.documentElement.style.fontSize = "16px";
      const before = [busy, numeric, idle].map((node) => Number.parseFloat(getComputedStyle(node).height));
      document.documentElement.style.fontSize = "24px";
      check([busy, numeric, idle].every((node, i) => Math.abs(Number.parseFloat(getComputedStyle(node).height) / before[i]! - 1.5) < 0.01), `Global font scales loading at surface unit ${unit}`);
    }
    const determinate = container.querySelector<HTMLElement>('[aria-label="Progress"]')!;
    check(determinate.getAttribute("aria-valuenow") === "65" && getComputedStyle(determinate).width === "24px" && getComputedStyle(determinate.querySelector("svg")!).display !== "none", "Determinate progress retains value, geometry and visible SVG");
    const paused = container.querySelector('[aria-label="Paused during swipe"]')!;
    check(getComputedStyle(paused).animationPlayState === "paused" && getComputedStyle(paused, "::after").animationPlayState === "paused", "Mobile swipe pauses loading motion");
    check(busy.getAttribute("role") === "progressbar" && !busy.hasAttribute("aria-valuenow"), "Busy status retains indeterminate accessibility semantics");
    return results;
  } finally {
    document.documentElement.style.fontSize = originalFont;
    document.documentElement.style.setProperty("--cowboy-desktop-size-unit", originalUnit);
    if (!keepFixture) {
      flushSync(() => root.unmount());
      container.remove();
    }
  }
}
