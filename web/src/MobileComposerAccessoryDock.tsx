import type { MouseEventHandler, PointerEventHandler, ReactNode } from "react";
import { alpha, Box, IconButton, Stack, Tooltip } from "@mui/material";
import { mobileComposerPanelFrameSx } from "./mobileComposerPrimitives";
import { NetworkIconButton } from "./NetworkActionFeedback";
import { useReliableTouchTap } from "./useReliableTouchTap";

/** A two-track keyboard-adjacent command surface for every focused Mobile editor. */
export function MobileComposerAccessoryDock({
  mode,
  formatActions,
  utilityActions,
  fixedAction,
  primaryLabel,
  primaryDisabled,
  onPrimary,
  primaryIcon,
  embedded = false,
}: {
  mode: "insert" | "selection";
  formatActions: ReactNode;
  utilityActions: ReactNode;
  fixedAction: ReactNode;
  primaryLabel: string;
  primaryDisabled: boolean;
  onPrimary: () => void;
  primaryIcon: ReactNode;
  /** Nest the two tracks inside an existing composer card. */
  embedded?: boolean;
}): React.JSX.Element {
  const primaryTap = useReliableTouchTap<HTMLButtonElement>(onPrimary);

  return (
    <Box
      data-mobile-composer-accessory
      data-toolbar-mode={mode}
      sx={{
        minHeight: 96,
        display: "grid",
        gridTemplateRows: "48px 48px",
        mx: embedded ? 0 : 1,
        // Keyboard separation belongs to the surface that positions the whole
        // composer. Keeping a second margin here doubled the gap in fullscreen
        // while embedded/inline compose still had no gap at all.
        mb: 0,
        ...(embedded
          ? { borderTop: 1, borderColor: "divider", overflow: "hidden" }
          : mobileComposerPanelFrameSx),
        borderColor: (theme) => alpha(theme.palette.divider, 0.5),
        bgcolor: "background.paper",
        boxShadow: embedded
          ? "none"
          : (theme) =>
            theme.palette.mode === "dark"
              ? "0 8px 26px rgba(0, 0, 0, 0.22)"
              : "0 8px 24px rgba(57, 42, 92, 0.08)",
        userSelect: "none",
        WebkitUserSelect: "none",
        animation: embedded
          ? "none"
          : "mobile-composer-dock-enter 180ms ease-out both",
        "@keyframes mobile-composer-dock-enter": {
          from: { opacity: 0, transform: "translateY(6px)" },
          to: { opacity: 1, transform: "translateY(0)" },
        },
        "@media (prefers-reduced-motion: reduce)": { animation: "none" },
      }}
    >
      <Stack
        data-mobile-composer-message-actions
        direction="row"
        alignItems="center"
        sx={{
          minWidth: 0,
          pl: 1.25,
          pr: 0.25,
          borderBottom: 1,
          borderColor: (theme) => alpha(theme.palette.divider, 0.34),
        }}
      >
        <Stack
          data-mobile-composer-utility-actions
          direction="row"
          alignItems="center"
          spacing={0.125}
          sx={{
            minWidth: 0,
            overflowX: "auto",
            overflowY: "hidden",
            overscrollBehaviorX: "contain",
            scrollbarWidth: "none",
            "&::-webkit-scrollbar": { display: "none" },
          }}
        >
          {utilityActions}
        </Stack>
        <Box sx={{ flex: 1, minWidth: 8 }} />
        <MobileComposerFixedActionSlot region="primary">
          <Tooltip title={primaryLabel}>
            <span>
              <IconButton
                aria-label={primaryLabel.toLowerCase()}
                disabled={primaryDisabled}
                onPointerDown={(event): void => {
                  event.preventDefault();
                  primaryTap.onPointerDown(event);
                }}
                onMouseDown={(event): void => event.preventDefault()}
                onPointerMove={primaryTap.onPointerMove}
                onPointerUp={primaryTap.onPointerUp}
                onPointerCancel={primaryTap.onPointerCancel}
                onClick={primaryTap.onClick}
                sx={{
                  width: 44,
                  height: 44,
                  flexShrink: 0,
                  color: "primary.main",
                  transition:
                    "color 120ms ease, opacity 120ms ease, transform 120ms ease",
                  "&:active": { transform: "scale(0.94)" },
                  "&.Mui-disabled": {
                    color: "text.disabled",
                    opacity: 0.44,
                  },
                  "& .MuiSvgIcon-root": { fontSize: "1.375rem" },
                }}
              >
                {primaryIcon}
              </IconButton>
            </span>
          </Tooltip>
        </MobileComposerFixedActionSlot>
      </Stack>

      <MobileComposerEditingBar
        actions={formatActions}
        fixedAction={fixedAction}
      />
    </Box>
  );
}

/**
 * The keyboard-nearest track shared by compact, pending, and fullscreen editors.
 * Formatting may scroll, but the high-frequency keyboard boundary never does.
 */
export function MobileComposerEditingBar({
  actions,
  fixedAction,
}: {
  actions: ReactNode;
  fixedAction: ReactNode;
}): React.JSX.Element {
  return (
    <Box
      data-mobile-composer-editing-bar
      sx={{ minWidth: 0, height: 48, display: "flex", alignItems: "center" }}
    >
      <Stack
        data-mobile-composer-format-actions
        direction="row"
        alignItems="center"
        spacing={0.125}
        sx={{
          flex: 1,
          minWidth: 0,
          height: 48,
          pl: 1.25,
          overflowX: "auto",
          overflowY: "hidden",
          overscrollBehaviorX: "contain",
          scrollbarWidth: "none",
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
        {actions}
      </Stack>
      <MobileComposerFixedActionSlot region="editing">
        {fixedAction}
      </MobileComposerFixedActionSlot>
    </Box>
  );
}

/** One shared right-edge geometry; only paired flowing tracks draw a divider. */
export function MobileComposerFixedActionSlot({
  children,
  region,
  overlay = false,
}: {
  children: ReactNode;
  region: "primary" | "editing";
  /** Pin the shared slot over the compact composer instead of flowing in a track. */
  overlay?: boolean;
}): React.JSX.Element {
  return (
    <Box
      data-mobile-composer-fixed-slot
      data-mobile-composer-utility-rail={overlay ? "" : undefined}
      data-mobile-composer-primary-actions={region === "primary"
        ? ""
        : undefined}
      data-mobile-composer-fixed-action={region === "editing" ? "" : undefined}
      sx={{
        flex: "0 0 48px",
        width: 48,
        height: 44,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        ml: 0.25,
        pr: 0.25,
        // The compact fullscreen control floats alone over the editor and needs
        // no rail separator. Expanded two-track docks retain aligned dividers.
        borderLeft: overlay ? 0 : 1,
        borderColor: (theme) => alpha(theme.palette.divider, 0.22),
        ...(overlay && {
          position: "absolute",
          top: 2,
          right: 0,
          zIndex: 2,
          // A flowing slot needs this gap after scrollable actions. An absolute
          // slot is already pinned to the card edge, so the margin would shift
          // its divider away from the editing track below.
          ml: 0,
        }),
      }}
    >
      {children}
    </Box>
  );
}

/** A stable 44pt accessory action whose glyph follows the global font scale. */
export function MobileComposerAccessoryButton({
  title,
  onClick,
  onPointerDown,
  onPointerMove,
  onPointerUp,
  onPointerCancel,
  networkAction,
  children,
  disabled = false,
  color = "default",
  preserveEditorFocus = true,
}: {
  title: string;
  onClick?: MouseEventHandler<HTMLButtonElement>;
  onPointerDown?: PointerEventHandler<HTMLButtonElement>;
  onPointerMove?: PointerEventHandler<HTMLButtonElement>;
  onPointerUp?: PointerEventHandler<HTMLButtonElement>;
  onPointerCancel?: PointerEventHandler<HTMLButtonElement>;
  networkAction?: () => Promise<void> | void;
  children: ReactNode;
  disabled?: boolean;
  color?: "default" | "primary" | "warning";
  /** Format actions keep the textarea first-responder. Hide-keyboard must not. */
  preserveEditorFocus?: boolean;
}): React.JSX.Element {
  const buttonSx = {
    width: 44,
    height: 44,
    flexShrink: 0,
    color: "text.secondary",
    ...(color === "primary" && { color: "primary.main" }),
    ...(color === "warning" && { color: "warning.main" }),
    "&:active": { transform: "scale(0.94)" },
    "&.Mui-disabled": { color: "text.disabled", opacity: 0.44 },
    "& .MuiSvgIcon-root": { fontSize: "1.375rem" },
  };
  return (
    <Tooltip title={title}>
      <span>
        {networkAction
          ? (
            <NetworkIconButton
              aria-label={title}
              color={color}
              disabled={disabled}
              networkAction={networkAction}
              onPointerDown={(event): void => {
                if (preserveEditorFocus) event.preventDefault();
                onPointerDown?.(event);
              }}
              onMouseDown={(event): void => {
                if (preserveEditorFocus) event.preventDefault();
              }}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerCancel={onPointerCancel}
              sx={buttonSx}
            >
              {children}
            </NetworkIconButton>
          )
          : (
            <IconButton
              aria-label={title}
              color={color}
              disabled={disabled}
              onPointerDown={(event): void => {
                if (preserveEditorFocus) event.preventDefault();
                onPointerDown?.(event);
              }}
              onMouseDown={(event): void => {
                if (preserveEditorFocus) event.preventDefault();
              }}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerCancel={onPointerCancel}
              onClick={onClick}
              sx={buttonSx}
            >
              {children}
            </IconButton>
          )}
      </span>
    </Tooltip>
  );
}
