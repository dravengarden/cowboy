import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CircularProgress, CssBaseline, Stack, Typography } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";
import { Sheet } from "./Sheet";
import { StatusDot } from "./SessionStatusDot";
import { SessionMachineBadge } from "./SessionMachineBadge";
import { SessionUpdateBadge } from "./SessionUpdateBadge";
import { mobileCompositorFlattenSx, mobilePresentationMovingRootSx } from "./mobilePresentationMotion";
import { splashHtml } from "../../components/app-shell/splash";

/** Real theme, synthetic activity, no product account. Keep mounted for CDP
 * screenshots and reduced-motion / forced-colors emulation. */
export async function runLoadingProgressBrowserConformance(keepFixture = false): Promise<string[]> {
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
        <Box
          data-mobile-product="agent"
          data-mobile-product-moving="true"
          sx={mobilePresentationMovingRootSx("data-mobile-product-moving")}
        >
          <Box data-mobile-review-loading>
            <CircularProgress size={20} thickness={3.6} aria-label="Cold review loading" />
          </Box>
        </Box>
        <Box data-mobile-product="agent">
          <CircularProgress size={16} thickness={3.6} aria-label="Mobile thin ring" />
          <CircularProgress size={16} thickness={5} aria-label="Mobile thick ring" />
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
    const coldReview = container.querySelector('[aria-label="Cold review loading"]')!;
    check(getComputedStyle(paused).animationPlayState === "paused" &&
      getComputedStyle(paused.querySelector("svg")!).animationName === "none" &&
      getComputedStyle(paused.querySelector("svg")!).transform === "none" &&
      getComputedStyle(paused.querySelector("svg")!).animationPlayState === "paused" &&
      getComputedStyle(paused.querySelector(".MuiCircularProgress-circle")!).animationName === "cowboy-loading-dash" &&
      getComputedStyle(paused.querySelector(".MuiCircularProgress-circle")!).animationPlayState === "paused" &&
      getComputedStyle(pausedStartup, "::before").animationPlayState === "paused" &&
      getComputedStyle(pausedStartup, "::after").animationPlayState === "paused" &&
      getComputedStyle(pausedStartup.querySelector("svg")!).animationPlayState === "paused" &&
      getComputedStyle(pausedRing).animationPlayState === "paused" &&
      getComputedStyle(pausedRing.querySelector("svg")!).animationName === "none" &&
      getComputedStyle(pausedRing.querySelector("svg")!).transform === "none" &&
      getComputedStyle(pausedRing.querySelector("svg")!).animationPlayState === "paused" &&
      getComputedStyle(pausedRing.querySelector(".MuiCircularProgress-circle")!).animationName === "cowboy-loading-dash" &&
      getComputedStyle(pausedRing.querySelector(".MuiCircularProgress-circle")!).animationPlayState === "paused" &&
      getComputedStyle(coldReview).animationPlayState === "paused" &&
      getComputedStyle(coldReview.querySelector("svg")!).animationName === "none" &&
      getComputedStyle(coldReview.querySelector("svg")!).transform === "none" &&
      getComputedStyle(coldReview.querySelector(".MuiCircularProgress-circle")!).animationName === "cowboy-loading-dash" &&
      getComputedStyle(coldReview.querySelector(".MuiCircularProgress-circle")!).animationPlayState === "paused" &&
      await (async () => {
        const host = document.createElement("div");
        document.body.append(host);
        const sheetRoot = createRoot(host);
        try {
          flushSync(() => sheetRoot.render(
            <SurfaceContext.Provider value={classifySurface({ finePointer: false, hover: false, tabletWidth: false, touchCapable: true, desktopHost: false })}>
              <BrowserProductTheme>
                <Sheet open onClose={() => {}} title="Loading" forceSheet cover portal>
                  <CircularProgress size={26} aria-label="Portaled sheet loading" />
                </Sheet>
              </BrowserProductTheme>
            </SurfaceContext.Provider>,
          ));
          let ring: HTMLElement | null = null;
          for (let attempt = 0; attempt < 40 && ring === null; attempt += 1) {
            ring = document.body.querySelector<HTMLElement>('[aria-label="Portaled sheet loading"]');
            if (ring === null) await new Promise<void>((resolve) => setTimeout(resolve, 16));
          }
          return ring !== null &&
            !container.contains(ring) &&
            !host.contains(ring) &&
            getComputedStyle(ring.querySelector("svg")!).animationName === "none" &&
            getComputedStyle(ring.querySelector("svg")!).transform === "none" &&
            getComputedStyle(ring.querySelector(".MuiCircularProgress-circle")!).animationName === "cowboy-loading-dash";
        } finally {
          flushSync(() => sheetRoot.unmount());
          host.remove();
        }
      })(), "Mobile swipe surfaces keep rings paint-only and pause loading motion");
    check(["Mobile thin ring", "Mobile thick ring"].every((label) => {
      const ring = container.querySelector<HTMLElement>(`[aria-label="${label}"]`)!;
      const arc = ring.querySelector<SVGCircleElement>(".MuiCircularProgress-circle")!;
      const circumference = 2 * Math.PI * arc.r.baseVal.value;
      const motion = arc.getAnimations()[0]!;
      motion.pause();
      try {
        motion.currentTime = 300;
        const offset = Number.parseFloat(getComputedStyle(arc).strokeDashoffset || "0");
        return Math.abs(offset + circumference / 2) < 0.25;
      } finally {
        motion.play();
      }
    }), "Mobile rings with different thicknesses animate to their own circumference without keyframe collision");
    check(busy.getAttribute("role") === "progressbar" && !busy.hasAttribute("aria-valuenow"), "Busy status retains indeterminate accessibility semantics");
    const starting = container.querySelector('[aria-label="Starting…"]')!;
    check(getComputedStyle(busy).animationName === "none" &&
      getComputedStyle(busy.querySelector("svg")!).animationName === "cowboy-loading-rotate" &&
      getComputedStyle(busy.querySelector(".MuiCircularProgress-circle")!).animationName === "none" &&
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
      const motion = svg.getAnimations()[0]!;
      const anchor = node.getBoundingClientRect();
      const circumference = 2 * Math.PI * arc.r.baseVal.value;
      motion.pause();
      try {
        return getComputedStyle(svg).overflow === "visible" && [0, 100, 200, 300, 450, 599].every((time) => {
          motion.currentTime = time;
          const svgStyle = getComputedStyle(svg);
          const style = getComputedStyle(arc);
          const fixed = track.getBoundingClientRect();
          const moving = arc.getBoundingClientRect();
          const pattern = style.strokeDasharray.split(/[, ]+/).map(Number.parseFloat);
          const expectedRad = (time / 600) * Math.PI * 2;
          const matrixMatch = svgStyle.transform.match(/^matrix\(([^)]+)\)$/);
          const rotationOk = time === 0
            ? svgStyle.transform === "none" || (matrixMatch !== null && Math.abs(Number.parseFloat(matrixMatch[1]!.split(",")[0]!) - 1) < 0.02)
            : matrixMatch !== null &&
              Math.abs(Number.parseFloat(matrixMatch[1]!.split(",")[0]!) - Math.cos(expectedRad)) < 0.02 &&
              Math.abs(Number.parseFloat(matrixMatch[1]!.split(",")[1]!) - Math.sin(expectedRad)) < 0.02;
          return rotationOk &&
            style.transform === "none" &&
            Math.abs(Number.parseFloat(style.strokeDashoffset || "0")) < 0.01 &&
            Math.abs(pattern[0]! - circumference / 3) < 0.01 &&
            Math.abs(pattern.reduce((sum, length) => sum + length, 0) - circumference) < 0.01 &&
            Math.abs(fixed.x + fixed.width / 2 - anchor.x - anchor.width / 2) < 0.1 &&
            Math.abs(fixed.y + fixed.height / 2 - anchor.y - anchor.height / 2) < 0.1 &&
            Math.abs(moving.x + moving.width / 2 - anchor.x - anchor.width / 2) < 0.1 &&
            Math.abs(moving.y + moving.height / 2 - anchor.y - anchor.height / 2) < 0.1;
        });
      } finally {
        motion.play();
      }
    }), "Status and sending rings rotate the full SVG on the compositor without strokeDashoffset layout churn or center drift");
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
