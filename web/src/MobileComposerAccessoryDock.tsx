import type { MouseEventHandler, ReactNode } from "react";
import { alpha, Box, IconButton, Stack, Tooltip } from "@mui/material";
import { mobileComposerPanelFrameSx } from "./mobileComposerPrimitives";

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
  return (
    <Box
      data-mobile-composer-accessory
      data-toolbar-mode={mode}
      sx={{
        minHeight: 96,
        display: "grid",
        gridTemplateRows: "48px 48px",
        mx: embedded ? 0 : 1,
        mb: embedded ? 0 : 0.75,
        ...(embedded
          ? { borderTop: 1, borderColor: "divider", overflow: "hidden" }
          : mobileComposerPanelFrameSx),
        borderColor: (theme) => alpha(theme.palette.divider, 0.5),
        bgcolor: (theme) =>
          alpha(
            theme.palette.background.paper,
            theme.palette.mode === "dark" ? 0.96 : 0.94,
          ),
        backdropFilter: "blur(18px) saturate(135%)",
        WebkitBackdropFilter: "blur(18px) saturate(135%)",
        boxShadow: embedded
          ? "none"
          : (theme) =>
            theme.palette.mode === "dark"
              ? "0 8px 26px rgba(0, 0, 0, 0.22)"
              : "0 8px 24px rgba(57, 42, 92, 0.08)",
        userSelect: "none",
        WebkitUserSelect: "none",
        animation: embedded ? "none" : "mobile-composer-dock-enter 180ms ease-out both",
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
        spacing={0.125}
        sx={{
          minWidth: 0,
          px: 0.75,
          borderBottom: 1,
          borderColor: (theme) => alpha(theme.palette.divider, 0.34),
        }}
      >
        {utilityActions}
        <Box sx={{ flex: 1, minWidth: 8 }} />
        <Tooltip title={primaryLabel}>
          <span>
            <IconButton
              aria-label={primaryLabel.toLowerCase()}
              disabled={primaryDisabled}
              onPointerDown={(event): void => event.preventDefault()}
              onClick={onPrimary}
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
      </Stack>

      <Stack
        data-mobile-composer-format-actions
        direction="row"
        alignItems="center"
        spacing={0.125}
        sx={{
          minHeight: 48,
          minWidth: 0,
          px: 0.75,
          overflowX: "auto",
          overscrollBehaviorX: "contain",
          scrollbarWidth: "none",
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
        {formatActions}
        <Box sx={{ flex: 1, minWidth: 8 }} />
        <Box
          data-mobile-composer-fixed-action
          sx={{ flex: "0 0 44px", width: 44, height: 44 }}
        >
          {fixedAction}
        </Box>
      </Stack>
    </Box>
  );
}

/** A stable 44pt accessory action whose glyph follows the global font scale. */
export function MobileComposerAccessoryButton({
  title,
  onClick,
  children,
  disabled = false,
  color = "default",
}: {
  title: string;
  onClick: MouseEventHandler<HTMLButtonElement>;
  children: ReactNode;
  disabled?: boolean;
  color?: "default" | "warning";
}): React.JSX.Element {
  return (
    <Tooltip title={title}>
      <IconButton
        aria-label={title}
        color={color}
        disabled={disabled}
        onPointerDown={(event): void => event.preventDefault()}
        onClick={onClick}
        sx={{
          width: 44,
          height: 44,
          flexShrink: 0,
          color: "text.secondary",
          ...(color === "warning" && { color: "warning.main" }),
          "&:active": { transform: "scale(0.94)" },
          "&.Mui-disabled": { color: "text.disabled", opacity: 0.44 },
          "& .MuiSvgIcon-root": { fontSize: "1.375rem" },
        }}
      >
        {children}
      </IconButton>
    </Tooltip>
  );
}
