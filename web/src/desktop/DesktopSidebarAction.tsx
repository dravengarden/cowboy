import { alpha, ButtonBase, type SxProps, type Theme } from "@mui/material";
import { forwardRef, type ReactNode } from "react";
import { HintTooltip } from "../HintTooltip";
import { DesktopContextShortcut } from "./commands/DesktopContextShortcut";
import { DESKTOP_INSET_RADIUS } from "./DesktopEmbeddedControl";

/**
 * One icon of the Desktop Sessions bar (VS Code's activity bar on top) or of
 * the collapsed Sessions rail: a quiet glyph whose leader slot appears beside
 * it while Sessions has focus, on hover, and the moment the leader is armed
 * (FOCUS.md "Leader"). At rest the bar is as calm as VS Code's; the tooltip
 * names the key too.
 */
export const DesktopSidebarAction = forwardRef<HTMLButtonElement, {
  label: string;
  shortcut: string;
  onClick: () => void;
  disabled?: boolean | undefined;
  /** Tooltip detail, for example why the action is unavailable. */
  detail?: ReactNode | undefined;
  placement?: "bottom" | "right" | undefined;
  /** Rail actions are square 36 px targets; header actions are 28 px. */
  size?: "header" | "rail" | undefined;
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
    size = "header",
    active = false,
    sx,
    children,
    ...data
  },
  ref,
): React.JSX.Element {
  const box = size === "rail" ? 36 : 28;
  return (
    <DesktopContextShortcut
      badge={shortcut}
      placement={size === "rail" ? "corner" : "below"}
      restingHidden
      enabled={!disabled}
    >
      <HintTooltip
        title={label}
        shortcut={shortcut}
        detail={detail}
        placement={placement}
        enterDelay={250}
      >
        {/* A disabled button fires no pointer events; the span keeps the hint. */}
        <span style={{ display: "inline-flex" }}>
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
                width: box,
                height: box,
                flexShrink: 0,
                borderRadius: `${DESKTOP_INSET_RADIUS + (size === "rail" ? 2 : 0)}px`,
                color: active ? "primary.main" : "text.secondary",
                fontSize: size === "rail" ? "1.375rem" : "1.125rem",
                position: "relative",
                // The active view carries VS Code's underline.
                ...(active && {
                  "&::after": {
                    content: '""',
                    position: "absolute",
                    left: 5,
                    right: 5,
                    bottom: -8,
                    height: 2,
                    borderRadius: 1,
                    bgcolor: "primary.main",
                  },
                }),
                WebkitAppRegion: "no-drag",
                "& .MuiSvgIcon-root": { fontSize: "inherit" },
                "&:hover": { color: "primary.main", bgcolor: "action.hover" },
                "&.Mui-disabled": { color: "text.disabled" },
                "&.Mui-focusVisible": {
                  boxShadow: (theme) => `inset 0 0 0 2px ${alpha(theme.palette.primary.main, 0.5)}`,
                },
              },
              ...(Array.isArray(sx) ? sx : [sx]),
            ]}
          >
            {children}
          </ButtonBase>
        </span>
      </HintTooltip>
    </DesktopContextShortcut>
  );
});
