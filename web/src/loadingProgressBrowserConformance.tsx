import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CircularProgress, CssBaseline, Stack, Typography } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { StatusDot } from "./SessionStatusDot";
import { SessionMachineBadge } from "./SessionMachineBadge";
import { SessionUpdateBadge } from "./SessionUpdateBadge";
import { mobileCompositorFlattenSx } from "./mobilePresentationMotion";
import { splashHtml } from "../../components/app-shell/splash";

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
        <Box data-machine-warning-preview>
          {[true, false].map((compact) => <SessionMachineBadge key={String(compact)} compact={compact}
            session={{ id: "warning", title: "Warning", provider: "claude-code", machine_id: "ovh", cwd: "/runtime", status: "interrupted", execution_binding: {} }} />)}
        </Box>
        <Stack direction="row" spacing={0.5} alignItems="center" data-session-badge-preview>
          <Typography variant="caption">columbus</Typography>
          <SessionMachineBadge compact session={{
            id: "remote", title: "Remote", provider: "codex", machine_id: "ovh", cwd: "/runtime", status: "running",
            execution_binding: {
              schema: 1,
              runtime: { machine_id: "ovh", cwd: "/runtime" },
              environment: { machine_id: "hawk", protocol: 1 },
              workspace: { cwd: "/target" },
            },
          }} />
          <SessionMachineBadge compact session={{ id: "local", title: "Local", provider: "codex", machine_id: "hawk", cwd: "/target", status: "running" }} />
          <SessionUpdateBadge session={{ id: "waiting", title: "Waiting", provider: "codex", cwd: "/target", status: "exited", resource_wait: "capacity" }} />
        </Stack>
        {(["busy", "starting", "running", "exited"] as const).map((status) => (
          <Stack key={status} direction="row" spacing={1.5} alignItems="center">
            <StatusDot status={status} /><Typography>{status}</Typography>
          </Stack>
        ))}
        <Stack direction="row" spacing={2} alignItems="center">
          {[11, 12, 16, 24, 40].map((size) => <CircularProgress key={size} size={size} thickness={size === 11 ? 5 : 3.6} aria-label={`Loading ${size}`} />)}
        </Stack>
        <Stack direction="row" spacing={2} alignItems="center">
          {[12, 14, 16, 24].map((size) => <CircularProgress key={size} data-session-activity="true" size={size} aria-label={`Working ${size}`} />)}
        </Stack>
        <CircularProgress variant="determinate" value={65} size={24} aria-label="Progress" />
        <Box sx={mobileCompositorFlattenSx}>
          <Box data-mobile-drawer-surface>
            <CircularProgress data-session-activity="true" size={16} aria-label="Paused during swipe" />
            <CircularProgress data-session-starting="true" size={16} aria-label="Paused startup" />
            <CircularProgress size={16} aria-label="Paused loading" />
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
    check(Array.from(container.querySelectorAll('[data-machine-warning-preview] .MuiChip-root')).every((chip) => {
      const icon = chip.querySelector(".MuiChip-icon")!.getBoundingClientRect();
      const bounds = chip.getBoundingClientRect();
      return icon.height < bounds.height && icon.top >= bounds.top && icon.bottom <= bounds.bottom;
    }), "Machine warning icons fit compact and regular badges at enlarged font size");
    check(determinate.getAttribute("aria-valuenow") === "65" && getComputedStyle(determinate).width === "24px" && getComputedStyle(determinate.querySelector("svg")!).display !== "none", "Determinate progress retains value, geometry and visible SVG");
    const paused = container.querySelector('[aria-label="Paused during swipe"]')!;
    const pausedStartup = container.querySelector('[aria-label="Paused startup"]')!;
    const pausedRing = container.querySelector('[aria-label="Paused loading"]')!;
    check(getComputedStyle(paused).animationPlayState === "paused" &&
      getComputedStyle(paused.querySelector(".MuiCircularProgress-circle")!).animationPlayState === "paused" &&
      getComputedStyle(pausedStartup, "::before").animationPlayState === "paused" &&
      getComputedStyle(pausedStartup, "::after").animationPlayState === "paused" &&
      getComputedStyle(pausedStartup.querySelector("svg")!).animationPlayState === "paused" &&
      getComputedStyle(pausedRing).animationPlayState === "paused" &&
      getComputedStyle(pausedRing.querySelector(".MuiCircularProgress-circle")!).animationPlayState === "paused", "Mobile swipe pauses loading motion");
    check(busy.getAttribute("role") === "progressbar" && !busy.hasAttribute("aria-valuenow"), "Busy status retains indeterminate accessibility semantics");
    const starting = container.querySelector('[aria-label="Starting…"]')!;
    check(getComputedStyle(busy).animationName === "none" &&
      getComputedStyle(busy.querySelector(".MuiCircularProgress-circle")!).animationName === "cowboy-loading-rotate" &&
      getComputedStyle(busy.querySelector("svg")!).display !== "none" &&
      getComputedStyle(busy.querySelector(".MuiCircularProgress-circle")!).display !== "none" &&
      getComputedStyle(numeric.querySelector(".MuiCircularProgress-circle")!).display !== "none" &&
      getComputedStyle(starting.querySelector(".MuiCircularProgress-circle")!).display === "none" &&
      getComputedStyle(starting, "::before").animationName === "cowboy-starting-wave", "Work and general waits use MUI rings; startup uses bars");
    check([busy, numeric].every((node) => {
      const track = node.querySelector(".MuiCircularProgress-track");
      return track !== null && getComputedStyle(track).display !== "none" &&
        Number.parseFloat(getComputedStyle(track).opacity) <= 0.25;
    }) && getComputedStyle(determinate.querySelector(".MuiCircularProgress-track")!).display === "none" &&
      getComputedStyle(starting.querySelector(".MuiCircularProgress-track")!).display === "none",
      "Faint full tracks show for waits without changing startup or determinate progress");
    check([busy, numeric, container.querySelector('[aria-label="Loading 11"]')!].every((node) => {
      const svg = node.querySelector("svg")!;
      const arc = node.querySelector<SVGCircleElement>(".MuiCircularProgress-circle")!;
      const track = node.querySelector(".MuiCircularProgress-track")!;
      const motion = arc.getAnimations()[0]!;
      const anchor = track.getBoundingClientRect();
      const circumference = 2 * Math.PI * arc.r.baseVal.value;
      motion.pause();
      try {
        return getComputedStyle(svg).overflow === "visible" && [0, 100, 200, 300, 450, 599].every((time) => {
          motion.currentTime = time;
          const style = getComputedStyle(arc);
          const fixed = track.getBoundingClientRect();
          const moving = arc.getBoundingClientRect();
          const pattern = style.strokeDasharray.split(/[, ]+/).map(Number.parseFloat);
          return style.transform === "none" &&
            Math.abs(pattern[0]! - circumference / 3) < 0.01 &&
            Math.abs(pattern.reduce((sum, length) => sum + length, 0) - circumference) < 0.01 &&
            Math.abs(Number.parseFloat(style.strokeDashoffset) + time / 600 * circumference) < 0.01 &&
            Math.abs(fixed.x - anchor.x) < 0.01 && Math.abs(fixed.y - anchor.y) < 0.01 &&
            Math.abs(moving.x + moving.width / 2 - anchor.x - anchor.width / 2) < 0.1 &&
            Math.abs(moving.y + moving.height / 2 - anchor.y - anchor.height / 2) < 0.1;
        });
      } finally {
        motion.play();
      }
    }), "Status and sending rings move paint through a full loop without transformed SVG clipping or track movement");
    const splash = splashHtml({ title: "Cowboy" });
    const shell = document.createElement("div");
    shell.innerHTML = splash.head + splash.body;
    document.body.append(shell);
    try {
      const mark = shell.querySelector(".app-splash-spinner")!;
      check(getComputedStyle(mark).animationName === "cowboy-splash-rotate" && getComputedStyle(mark).animationDuration === "0.6s" && /conic-gradient\(.* 120deg,/.test(getComputedStyle(mark).backgroundImage) && getComputedStyle(mark).maskImage.includes("radial-gradient"), "Pre-mount splash uses a one-third arc over a faint full track");
    } finally {
      shell.remove();
    }
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
