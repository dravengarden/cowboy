import type { ReactNode } from "react";
import { alpha, Box, IconButton, Stack, Tooltip } from "@mui/material";

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
}: {
  mode: "insert" | "selection";
  formatActions: ReactNode;
  utilityActions: ReactNode;
  fixedAction: ReactNode;
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
        minHeight: 96,
        display: "grid",
        gridTemplateRows: "48px 48px",
        borderTop: 1,
        borderColor: (theme) => alpha(theme.palette.divider, 0.42),
        bgcolor: (theme) =>
          alpha(
            theme.palette.background.paper,
            theme.palette.mode === "dark" ? 0.94 : 0.92,
          ),
        backdropFilter: "blur(18px) saturate(135%)",
        WebkitBackdropFilter: "blur(18px) saturate(135%)",
        boxShadow: "none",
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
          minHeight: 48,
          minWidth: 0,
          px: 0.75,
          borderBottom: 1,
          borderColor: (theme) => alpha(theme.palette.divider, 0.34),
          overflowX: "auto",
          overscrollBehaviorX: "contain",
          scrollbarWidth: "none",
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
        {formatActions}
      </Stack>

      <Stack
        data-mobile-composer-message-actions
        direction="row"
        alignItems="center"
        spacing={0.125}
        sx={{ minWidth: 0, px: 0.75 }}
      >
        {utilityActions}
        <Box sx={{ flex: 1, minWidth: 8 }} />
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
        <Box
          data-mobile-composer-fixed-action
          sx={{
            position: "relative",
            flex: "0 0 45px",
            width: 45,
            height: 44,
            ml: 0.25,
            pl: 0.25,
            "&::before": {
              content: '""',
              position: "absolute",
              left: 0,
              top: 10,
              bottom: 10,
              width: "1px",
              borderRadius: 1,
              bgcolor: (theme) => alpha(theme.palette.divider, 0.42),
            },
          }}
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
