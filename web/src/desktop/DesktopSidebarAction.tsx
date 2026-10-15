import { alpha, ButtonBase, type SxProps, type Theme } from "@mui/material";
import { forwardRef, type ReactNode } from "react";
import { HintTooltip } from "../HintTooltip";
import { DesktopShortcut } from "./commands/DesktopKeycap";
import { DESKTOP_INSET_RADIUS } from "./DesktopEmbeddedControl";

/**
 * One entry of the Desktop Sessions bar (VS Code's activity bar on top) or of
 * the collapsed Sessions rail: the glyph with its leader slot stacked under
 * it, inside the control. Stacking keeps every key visible at rest (FOCUS.md
 * "Leader") while a whole row of entries still fits the narrowest sidebar,
 * and nothing hangs outside the bar to be clipped or to cover the list. The
 * slot lights with the armed leader like every other slot.
 */
export const DesktopSidebarAction = forwardRef<HTMLButtonElement, {
  label: string;
  shortcut: string;
  onClick: () => void;
  disabled?: boolean | undefined;
  /** Tooltip detail, for example why the action is unavailable. */
  detail?: ReactNode | undefined;
  placement?: "bottom" | "right" | undefined;
  /** The view this bar is showing (VS Code's active activity item). */
  active?: boolean | undefined;
  sx?: SxProps<Theme> | undefined;
  children: ReactNode;
} & Record<`data-${string}`, string | undefined>>(function DesktopSidebarAction(
  {
    label,
    shortcut,
    onClick,
    disabled = false,
    detail,
    placement = "bottom",
    active = false,
    sx,
    children,
    ...data
  },
  ref,
): React.JSX.Element {
  return (
    <HintTooltip
      title={label}
      shortcut={shortcut}
      detail={detail}
      placement={placement}
      enterDelay={350}
    >
      {/* A disabled button fires no pointer events; the span keeps the hint. */}
      <span style={{ display: "inline-flex", flexShrink: 0 }}>
        <ButtonBase
          {...data}
          ref={ref}
          aria-label={label}
          aria-keyshortcuts={shortcut}
          aria-current={active ? "page" : undefined}
          disabled={disabled}
          // Keep focus where it was: these act on the workspace, not on
          // themselves, exactly like their leader keys.
          onMouseDown={(event): void => event.preventDefault()}
          onClick={onClick}
          sx={[
            {
              position: "relative",
              minWidth: 32,
              height: 40,
              px: 0.25,
              flexDirection: "column",
              justifyContent: "center",
              gap: "2px",
              borderRadius: `${DESKTOP_INSET_RADIUS}px`,
              color: active ? "primary.main" : "text.secondary",
              WebkitAppRegion: "no-drag",
              "& > .MuiSvgIcon-root": { fontSize: "1.05rem" },
              "&:hover": { color: "primary.main", bgcolor: "action.hover" },
              "&.Mui-disabled": { color: "text.disabled", opacity: 0.7 },
              "&.Mui-focusVisible": {
                boxShadow: (theme) => `inset 0 0 0 2px ${alpha(theme.palette.primary.main, 0.5)}`,
              },
              // The shown view carries VS Code's underline on the bar's edge.
              ...(active && {
                "&::after": {
                  content: '""',
                  position: "absolute",
                  left: 6,
                  right: 6,
                  bottom: -2,
                  height: 2,
                  borderRadius: 1,
                  bgcolor: "primary.main",
                },
              }),
            },
            ...(Array.isArray(sx) ? sx : [sx]),
          ]}
        >
          {children}
          <DesktopShortcut
            shortcut={shortcut}
            compact
            availability={disabled ? "inactive" : "available"}
          />
        </ButtonBase>
      </span>
    </HintTooltip>
  );
});
