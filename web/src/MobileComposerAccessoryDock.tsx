import type { ReactNode } from "react";
import { alpha, Box, IconButton, Stack, Tooltip } from "@mui/material";

/** A single keyboard-adjacent command surface for every focused Mobile editor. */
export function MobileComposerAccessoryDock({
  mode,
  formatActions,
  utilityActions,
  primaryLabel,
  primaryDisabled,
  onPrimary,
  primaryIcon,
}: {
  mode: "insert" | "selection";
  formatActions: ReactNode;
  utilityActions: ReactNode;
  primaryLabel: string;
  primaryDisabled: boolean;
  onPrimary: () => void;
  primaryIcon: ReactNode;
}): React.JSX.Element {
  return (
    <Box
      data-mobile-composer-accessory
      data-toolbar-mode={mode}
      sx={{
        minHeight: 52,
        display: "flex",
        alignItems: "center",
        gap: 0.5,
        px: 0.75,
        py: 0.5,
        borderTop: 1,
        borderColor: "divider",
        bgcolor: (theme) =>
          alpha(theme.palette.background.paper, theme.palette.mode === "dark" ? 0.9 : 0.86),
        backdropFilter: "blur(18px) saturate(135%)",
        WebkitBackdropFilter: "blur(18px) saturate(135%)",
        boxShadow: (theme) =>
          `0 -8px 24px ${alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.14 : 0.045)}`,
        userSelect: "none",
        WebkitUserSelect: "none",
      }}
    >
      <Stack
        data-mobile-composer-format-actions
        direction="row"
        alignItems="center"
        spacing={0.125}
        sx={{
          flex: "1 1 auto",
          minWidth: 0,
          overflowX: "auto",
          overscrollBehaviorX: "contain",
          scrollbarWidth: "none",
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
        {formatActions}
      </Stack>

      <Stack
        data-mobile-composer-utility-actions
        direction="row"
        alignItems="center"
        spacing={0.125}
        sx={{
          flex: "0 0 auto",
          minWidth: 0,
        }}
      >
        {utilityActions}
        <Tooltip title={primaryLabel}>
          <span>
            <IconButton
              aria-label={primaryLabel.toLowerCase()}
              disabled={primaryDisabled}
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
    </Box>
  );
}

/** A stable 44pt accessory action whose glyph follows the global font scale. */
export function MobileComposerAccessoryButton({
  title,
  onClick,
  children,
}: {
  title: string;
  onClick: () => void;
  children: ReactNode;
}): React.JSX.Element {
  return (
    <Tooltip title={title}>
      <IconButton
        aria-label={title}
        onClick={onClick}
        sx={{
          width: 44,
          height: 44,
          flexShrink: 0,
          color: "text.secondary",
          "&:active": { transform: "scale(0.94)" },
          "& .MuiSvgIcon-root": { fontSize: "1.375rem" },
        }}
      >
        {children}
      </IconButton>
    </Tooltip>
  );
}
