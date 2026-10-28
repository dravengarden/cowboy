import { alpha, Box, Tooltip, type TooltipProps } from "@mui/material";
import {
  cloneElement,
  type PointerEvent as ReactPointerEvent,
  type ReactElement,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from "react";
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
 *
 * A controlled hint also closes itself when the pointer is seen outside its
 * control or the window loses focus. MUI closes only on that control's own
 * mouseleave, which a pointer capture, a fast exit or an app switch can skip,
 * leaving the hint standing over unrelated content.
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
  const anchor = useRef<Element | null>(null);
  useEffect(() => {
    if (suppressed) setHovered(false);
  }, [suppressed]);
  useEffect(() => {
    if (!hovered) return undefined;
    const leave = (event: Event): void => {
      const target = event.target instanceof Node ? event.target : null;
      if (!target || !anchor.current?.contains(target)) setHovered(false);
    };
    const blur = (): void => setHovered(false);
    document.addEventListener("pointerover", leave, true);
    document.addEventListener("pointerdown", leave, true);
    globalThis.addEventListener("blur", blur);
    return () => {
      document.removeEventListener("pointerover", leave, true);
      document.removeEventListener("pointerdown", leave, true);
      globalThis.removeEventListener("blur", blur);
    };
  }, [hovered]);
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
      {suppressed === undefined ? children : cloneElement(
        children as ReactElement<{
          onPointerEnter?: (event: ReactPointerEvent<Element>) => void;
        }>,
        {
          // Recorded synchronously: MUI opens after its enter delay, when the
          // event no longer carries its currentTarget.
          onPointerEnter: (event: ReactPointerEvent<Element>): void => {
            anchor.current = event.currentTarget;
            (children as ReactElement<{
              onPointerEnter?: (event: ReactPointerEvent<Element>) => void;
            }>).props.onPointerEnter?.(event);
          },
        },
      )}
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
