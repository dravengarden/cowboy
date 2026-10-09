import { useEffect, useState } from "react";
import { Box, CircularProgress, Tooltip } from "@mui/material";
import type { SxProps, Theme } from "@mui/material";
import { Circle } from "@mui/icons-material";
import type { ProviderUpdate, Status } from "./protocol";
import {
  backgroundProviderUpdateLabel,
  backgroundTasksLabel,
  providerUpdateProgress,
  waitingOnBackground,
} from "./backgroundActivity";
import { desktopSize } from "./surface/desktopSize";

// Status is shown as a single color-coded dot/spinner (no text label), so the
// hue has to carry the whole meaning. The palette tokens are chosen so the
// colors read the same here and in any future status surface:
//   green (success)       — live:     running (idle, ready) or busy (a turn in
//                                      flight). busy renders the green as a spinner.
//   blue  (info)          — starting:  process spinning up, not ready yet (spinner).
//   grey  (text.disabled) — dormant:   exited cleanly + resumable — "asleep, wakes
//                                       on resume". Deliberately NOT a warning hue.
//   amber (warning)       — interrupted: a turn was cut off by a daemon restart —
//                                       unfinished, needs attention. Amber (not the
//                                       crashed red) reads as "incomplete", not "dead".
//   red   (error)         — crashed:   died abnormally, can't reply.
export function statusColor(s: Status): string {
  switch (s) {
    case "running":
    case "busy":
      return "success.main";
    case "starting":
      return "info.main";
    case "exited":
      return "text.disabled";
    case "interrupted":
      return "warning.main";
    case "crashed":
      return "error.main";
  }
}

// Human-readable meaning for the dot — surfaced in its tooltip / aria-label,
// since the dot itself is just color. Mirrors the statusColor mapping above.
export function statusLabel(s: Status): string {
  switch (s) {
    case "running":
      return "Live";
    case "busy":
      return "Running…";
    case "starting":
      return "Starting…";
    case "exited":
      return "Dormant";
    case "interrupted":
      return "Interrupted";
    case "crashed":
      return "Crashed";
  }
}

// One status indicator, shared by the header and the sidebar list so the
// "green = live/running, grey = dormant" code reads identically everywhere. The
// dot encodes state by color; the tooltip + aria-label spell it out for hover and
// assistive tech (touch has no hover, but the wording is also redundant with
// the surrounding chrome).
export function StatusDot({
  status,
  backgroundTasks,
  providerUpdate,
  sx,
}: {
  status: Status;
  backgroundTasks?: number | undefined;
  providerUpdate?: ProviderUpdate | undefined;
  sx?: SxProps<Theme>;
}): React.JSX.Element {
  const extra = Array.isArray(sx) ? sx : sx ? [sx] : [];
  // "busy" and "starting" are the *active* states — a turn is in flight, or the
  // process is spinning up — so a static dot would read as idle/stuck. Render the
  // active states as a tiny spinner sized to the dot, with the stroke following
  // statusColor for palette continuity. running / exited / crashed are settled,
  // so they stay a color-coded dot. `color="inherit"` lets the sx `color`
  // (statusColor) drive the stroke instead of a fixed MUI palette slot.
  // An idle session whose agent still waits on its own background work is
  // not settled either: it resumes on that work's result without a prompt.
  const waiting = waitingOnBackground(status, backgroundTasks);
  // An unattended Provider update is starting without anyone waiting on it;
  // it fills a determinate ring instead of spinning like a cold start.
  const updating = backgroundProviderUpdateLabel(status, providerUpdate);
  if (updating && providerUpdate) {
    return (
      <ProviderUpdateRing
        label={updating}
        startedAtMs={providerUpdate.started_at_ms}
        sx={sx}
      />
    );
  }
  const shown: Status = waiting ? "busy" : status;
  const label = waiting
    ? backgroundTasksLabel(backgroundTasks ?? 0)
    : statusLabel(status);
  const active = shown === "busy" || shown === "starting";
  const indicator = active
    ? (
      <CircularProgress
        size={desktopSize(11)}
        thickness={6}
        disableShrink
        color="inherit"
        aria-label={label}
        sx={[{ flexShrink: 0, color: statusColor(shown) }, ...extra]}
      />
    )
    : (
      <Circle
        aria-label={label}
        sx={[
          {
            fontSize: desktopSize(10),
            flexShrink: 0,
            color: statusColor(shown),
          },
          ...extra,
        ]}
      />
    );
  return (
    <Tooltip title={label} enterDelay={300}>
      {indicator}
    </Tooltip>
  );
}

// Determinate twin of the startup spinner: same size and info blue, a faint
// full track, and an estimated fill that advances once a second.
function ProviderUpdateRing({
  label,
  startedAtMs,
  sx,
}: {
  label: string;
  startedAtMs: number;
  sx?: SxProps<Theme> | undefined;
}): React.JSX.Element {
  const extra = Array.isArray(sx) ? sx : sx ? [sx] : [];
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, []);
  const progress = providerUpdateProgress(startedAtMs, now);
  const size = desktopSize(11);
  return (
    <Tooltip title={`${label} (~${String(progress)}%)`} enterDelay={300}>
      <Box
        role="progressbar"
        aria-label={label}
        aria-valuenow={progress}
        aria-valuemin={0}
        aria-valuemax={100}
        sx={[
          {
            position: "relative",
            display: "inline-flex",
            flexShrink: 0,
            width: size,
            height: size,
            color: statusColor("starting"),
          },
          ...extra,
        ]}
      >
        <CircularProgress
          variant="determinate"
          value={100}
          size={size}
          thickness={6}
          color="inherit"
          aria-hidden
          sx={{ position: "absolute", inset: 0, opacity: 0.25 }}
        />
        <CircularProgress
          variant="determinate"
          value={progress}
          size={size}
          thickness={6}
          color="inherit"
          aria-hidden
          sx={{ position: "absolute", inset: 0 }}
        />
      </Box>
    </Tooltip>
  );
}
