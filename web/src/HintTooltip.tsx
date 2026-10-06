import { alpha, Box, Tooltip, type TooltipProps } from "@mui/material";
import { type ReactNode, useEffect, useState } from "react";
import { desktopShortcutCaps } from "./desktop/commands/DesktopKeycap";

/**
 * The one hover hint. Every pointer hint is an MUI Tooltip; never a native
 * `title` attribute, whose OS bubble ignores the theme, waits ~1s and renders
 * shortcuts as raw registration strings (`Mod+K → W → R`).
 *
 * `title` is the action, `shortcut` a registration string from
 * `DESKTOP_SHORTCUTS` (drawn as keycaps), `detail` a secondary line.
 *
 * `suppressed` hides the hint while a gesture owns the control (a splitter
 * drag). Never toggle `disableHoverListener` for that: MUI then drops the
 * mouseleave handler of an already open tooltip, which stays open until the
 * pointer happens to hover that control again. MUI only reports onOpen and
 * onClose across a change of the `open` it was given, so a suppressed hint
 * forgets its hover and shows again on the next one.
 */
export function HintTooltip({
  title,
  shortcut,
  detail,
  suppressed,
  children,
  ...props
}: Omit<TooltipProps, "title"> & {
  title: ReactNode;
  shortcut?: string | undefined;
  detail?: ReactNode | undefined;
  suppressed?: boolean | undefined;
}): React.JSX.Element {
  const [hovered, setHovered] = useState(false);
  useEffect(() => {
    if (suppressed) setHovered(false);
  }, [suppressed]);
  const controlled = suppressed === undefined ? {} : {
    open: hovered && !suppressed,
    onOpen: (event: React.SyntheticEvent): void => {
      if (suppressed) return;
      setHovered(true);
      props.onOpen?.(event);
    },
    onClose: (event: Event | React.SyntheticEvent): void => {
      setHovered(false);
      props.onClose?.(event);
    },
  };
  return (
    <Tooltip
      {...props}
      {...controlled}
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
