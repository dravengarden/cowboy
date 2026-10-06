import { alpha, Box, Tooltip, type TooltipProps } from "@mui/material";
import type { ReactNode } from "react";
import { desktopShortcutCaps } from "./desktop/commands/DesktopKeycap";

/**
 * The one hover hint. Every pointer hint is an MUI Tooltip; never a native
 * `title` attribute, whose OS bubble ignores the theme, waits ~1s and renders
 * shortcuts as raw registration strings (`Mod+K → W → R`).
 *
 * `title` is the action, `shortcut` a registration string from
 * `DESKTOP_SHORTCUTS` (drawn as keycaps), `detail` a secondary line.
 */
export function HintTooltip({
  title,
  shortcut,
  detail,
  children,
  ...props
}: Omit<TooltipProps, "title"> & {
  title: ReactNode;
  shortcut?: string | undefined;
  detail?: ReactNode | undefined;
}): React.JSX.Element {
  return (
    <Tooltip
      {...props}
      title={title === "" || title == null
        ? ""
        : <HintContent title={title} shortcut={shortcut} detail={detail} />}
    >
      {children}
    </Tooltip>
  );
}

function HintContent({
  title,
  shortcut,
  detail,
}: {
  title: ReactNode;
  shortcut: string | undefined;
  detail: ReactNode;
}): React.JSX.Element {
  return (
    <Box sx={{ display: "grid", gap: 0.375, py: 0.125 }}>
      <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
        <Box component="span" sx={{ minWidth: 0, whiteSpace: "pre-line" }}>
          {title}
        </Box>
        {shortcut && (
          <Box
            component="span"
            aria-label={shortcut}
            sx={{
              display: "inline-flex",
              gap: 0.375,
              ml: "auto",
              flexShrink: 0,
            }}
          >
            {desktopShortcutCaps(shortcut).map((cap, index) => (
              <TooltipKeycap key={`${cap}-${String(index)}`} label={cap} />
            ))}
          </Box>
        )}
      </Box>
      {detail && (
        <Box component="span" sx={{ opacity: 0.74, fontSize: "0.92em" }}>
          {detail}
        </Box>
      )}
    </Box>
  );
}

/** A keycap tuned for the tooltip's dark surface in both theme modes. */
function TooltipKeycap({ label }: { label: string }): React.JSX.Element {
  return (
    <Box
      component="kbd"
      aria-hidden
      sx={{
        display: "inline-grid",
        placeItems: "center",
        minWidth: "1.25rem",
        height: "1.125rem",
        px: "0.25rem",
        borderRadius: "0.35rem",
        border: 1,
        borderColor: (theme) => alpha(theme.palette.common.white, 0.3),
        bgcolor: (theme) => alpha(theme.palette.common.white, 0.1),
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
        fontSize: "0.625rem",
        fontWeight: 750,
        lineHeight: 1,
        whiteSpace: "nowrap",
      }}
    >
      {label}
    </Box>
  );
}
