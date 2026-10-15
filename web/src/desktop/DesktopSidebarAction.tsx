import { alpha, ButtonBase, type SxProps, type Theme } from "@mui/material";
import { forwardRef, type ReactNode } from "react";
import { HintTooltip } from "../HintTooltip";
import { desktopSize } from "../surface/desktopSize";
import { LeaderKeycap } from "./commands/DesktopKeycap";
import { DESKTOP_WORKSPACE_PREFIX } from "./commands/workspaceShortcuts";
import { DESKTOP_INSET_RADIUS } from "./DesktopEmbeddedControl";

/**
 * Sx for a bar of sidebar entries: when the bar is narrower than `fitRem`
 * (everything scales with the global font size, so the fit is in rem) the
 * entries keep their glyphs and drop their slots instead of being clipped.
 */
export function sidebarSlotFitSx(fitRem: number) {
  return {
    containerType: "inline-size",
    [`@container (max-width: ${String(fitRem)}rem)`]: {
      "& .cowboy-sidebar-slot": { display: "none !important" },
    },
  } as const;
}

/** `Mod+K → S → S` → `SS`: the leader path a slot draws. */
function leaderPath(shortcut: string): string {
  const [prefix, ...strokes] = shortcut.split(" → ");
  return prefix === DESKTOP_WORKSPACE_PREFIX ? strokes.join("") : shortcut;
}

/**
 * One entry of the Desktop Sessions bars (VS Code's activity bar on top and
 * its view title bar) or of the collapsed Sessions rail. It is the same
 * control as a pane header's fold button: a 16-unit glyph and its quiet
 * leader slot side by side in a 24-unit capsule, so every entry, the fold
 * included, shares one height, one gap and one keycap. All of it is sized in
 * `desktopSize` units and follows the global font size.
 */
export const DesktopSidebarAction = forwardRef<HTMLButtonElement, {
  label: string;
  shortcut: string;
  onClick: () => void;
  disabled?: boolean | undefined;
  /** Tooltip detail, for example why the action is unavailable. */
  detail?: ReactNode | undefined;
  placement?: "bottom" | "right" | undefined;
  /** The view the bar is showing: VS Code's active activity item. */
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
              height: desktopSize(24),
              px: desktopSize(4),
              gap: desktopSize(3),
              flexShrink: 0,
              borderRadius: `${DESKTOP_INSET_RADIUS}px`,
              color: active ? "primary.main" : "text.secondary",
              border: 1,
              borderColor: "transparent",
              WebkitAppRegion: "no-drag",
              transition: "background-color 120ms ease, border-color 120ms ease, color 120ms ease",
              "& > .MuiSvgIcon-root": {
                fontSize: desktopSize(16),
                width: desktopSize(16),
                height: desktopSize(16),
              },
              "&:hover": {
                color: "primary.main",
                borderColor: (theme) => alpha(theme.palette.primary.main, 0.28),
                bgcolor: (theme) => alpha(theme.palette.primary.main, 0.06),
              },
              "&.Mui-disabled": { color: "text.disabled" },
              "&.Mui-focusVisible": {
                borderColor: "primary.main",
                boxShadow: (theme) => `0 0 0 2px ${alpha(theme.palette.primary.main, 0.16)}`,
              },
            },
            ...(Array.isArray(sx) ? sx : [sx]),
          ]}
        >
          {children}
          {/* A host too narrow for every slot at a large font size hides the
              slots (SIDEBAR_SLOT_FIT) rather than clip its entries; the
              tooltip, which-key and the palette still name every key. */}
          <span className="cowboy-sidebar-slot" style={{ display: "inline-flex" }}>
            <LeaderKeycap
              leaderKey={leaderPath(shortcut)}
              {...(disabled ? { availability: "inactive" as const } : {})}
            />
          </span>
          {active && (
            // VS Code's active-view underline, on the bar's bottom edge: the
            // 24-unit capsule sits centred in the 44-unit bar.
            <span
              aria-hidden
              style={{
                position: "absolute",
                left: 4,
                right: 4,
                bottom: `calc(-1 * ${desktopSize(10)} - 1px)`,
                height: 2,
                borderRadius: 1,
                background: "currentColor",
              }}
            />
          )}
        </ButtonBase>
      </span>
    </HintTooltip>
  );
});
